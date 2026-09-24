/// <reference path="./types.d.ts" />
// team-board's server-side logic: three things live here.
//
//   1. `onRecordCreate`/`onRecordUpdate` on `cards` — derives `searchText`
//      (the vector field's `sourceField`, see schema.json's `embedding`
//      field: auto-embed configs take exactly one source field, so
//      "embed from title *and* description" has to be a real column
//      something writes, not a config option) and enforces that a *real*
//      card (not a throwaway search query, see below) has a column —
//      `columnRef` is optional at the schema level (a `isQuery` row has
//      none) so this has to be a hook, not `required: true`.
//
//      CAVEAT (a real Cratebase limitation, live-verified building this
//      example — see the repo PR/commit description's "Cratebase bugs/
//      friction" section): `apply_embeddings` (crates/server/src/
//      embeddings.rs) runs in `routes::records::create_record`/
//      `update_record` *before* any JS hook gets to run
//      (crates/server/src/routes/records.rs:554,679) — so `searchText`
//      as set *here* is too late to affect that request's own embedding;
//      it only fixes up the stored value for any write this hook didn't
//      already cover. `src/hooks/useCards.ts` and `src/components/
//      CardDetail.tsx` work around this by sending `searchText` directly
//      in the create/update body from the client, which is what actually
//      makes the embedding populate. This hook still runs as a defensive
//      fallback (e.g. a direct dashboard edit that doesn't know about
//      `searchText` at all).
//   2. `onRecordAfterCreateSuccess`/`onRecordAfterUpdateSuccess` on
//      `cards` — when a card gets (re)assigned, writes a `notifications`
//      row and emails the assignee (lands in the dev mail inbox — see
//      README.md).
//   3. A `cronAdd` overdue-card flagger — once a minute (croner's finest
//      resolution), finds cards past `dueAt` that haven't been flagged
//      yet, notifies the assignee, and marks them flagged so it only
//      fires once per card.
//
// Semantic search itself (`?nearestTo=`) needs no hook at all: any team
// member already has `create`/`delete` on `cards` (see schema.json), so
// the app creates a throwaway `isQuery: true` card to get a real,
// same-pipeline embedding out of the update it triggers, ranks with
// `nearestTo`, then deletes it — entirely from `src/hooks/useSearch.ts`.
// That row is invisible to every client (including its own creator) in
// both `list` and realtime, because `isQuery = false` is baked into
// `cards`' `listRule`/`viewRule` and realtime delivery is gated by the
// exact same rule.

function computeSearchText(record) {
  const title = record.getString("title");
  const description = record.getString("description");
  return [title, description].filter((s) => s).join("\n\n");
}

onRecordCreate((e) => {
  e.record.set("searchText", computeSearchText(e.record));
  if (!e.record.get("isQuery") && !e.record.get("columnRef")) {
    throw new BadRequestError("columnRef is required.");
  }
  e.next();
}, "cards");

onRecordUpdate((e) => {
  e.record.set("searchText", computeSearchText(e.record));
  e.next();
}, "cards");

// ---------------------------------------------------------------------
// Team owner bootstrap (works around a real Cratebase timing bug)
// ---------------------------------------------------------------------
//
// Cratebase already has a reactive hook that inserts a team's owner into
// `_team_members` when `_teams` gets a new row (`crates/server/src/
// teams.rs`), gated on `settings.teams.enabled`. Live-verified building
// this example: that gate is only read once, at server *bootstrap*
// (`crates/server/src/app.rs`'s call to `crate::teams::bind_hooks`) — so
// turning it on at runtime via `PATCH /api/settings` (which scripts/
// setup.sh does, since the server is already running by the time setup
// runs — see scripts/dev.ts) does *not* retroactively bind that hook for
// the still-running process. A `_teams` row created afterwards, in that
// same process lifetime, is left ownerless: exactly what happened
// seeding this example the first time, and what would silently break the
// in-app "+ New team" flow (`src/hooks/useTeams.ts`'s `createTeam`) for
// anyone who didn't happen to restart the server after enabling teams.
// This hook is this app's own, always-bound backstop for that gap —
// idempotent against the built-in one via the `existing` check below, so
// it's a harmless no-op on a server where the built-in hook did fire.
onRecordAfterCreateSuccess((e) => {
  try {
    const ownerId = e.record.get("ownerRef");
    if (!ownerId) return;
    const existing = $app.findFirstRecordByFilter(
      "_team_members",
      "teamRef = {:team} && userRef = {:user}",
      { team: e.record.id, user: ownerId },
    );
    if (existing) return;
    const teamMembers = $app.findCollectionByNameOrId("_team_members");
    const membership = new Record(teamMembers, { teamRef: e.record.id, userRef: ownerId, role: "owner" });
    e.app.save(membership);
  } catch (err) {
    $app.logger().error("team-board: failed to bootstrap team owner membership", "team", e.record.id, "error", String(err));
  }
}, "_teams");

// ---------------------------------------------------------------------
// Assignment notifications
// ---------------------------------------------------------------------

function notifyAssignment(e, assigneeId) {
  try {
    const assignee = $app.findRecordById("users", assigneeId);
    if (!assignee) return;

    const notifications = $app.findCollectionByNameOrId("notifications");
    const notification = new Record(notifications, {
      userRef: assigneeId,
      teamRef: e.record.get("teamRef"),
      cardRef: e.record.id,
      message: 'You were assigned to "' + e.record.getString("title") + '"',
      read: false,
    });
    // Same transaction as the card's own still-open create/update (see
    // the module doc on `crates/jsvm`'s `onRecordAfterUpdateSuccess`) —
    // `e.app`, not `$app`, is what makes this transactional.
    e.app.save(notification);

    const mail = $app.newMailClient();
    mail.send({
      to: [{ address: assignee.getString("email"), name: assignee.getString("name") }],
      subject: 'You were assigned: ' + e.record.getString("title"),
      text:
        e.record.getString("title") +
        "\n\n" +
        (e.record.getString("description") || "(no description)") +
        "\n\nOpen team-board to see it on the board.",
      html:
        "<p><strong>" +
        e.record.getString("title") +
        "</strong></p><p>" +
        (e.record.getString("description") || "(no description)") +
        "</p><p>Open team-board to see it on the board.</p>",
    });
  } catch (err) {
    $app.logger().error("team-board: assignment notification failed", "card", e.record.id, "error", String(err));
  }
}

onRecordAfterCreateSuccess((e) => {
  const assigneeId = e.record.get("assigneeRef");
  if (assigneeId) notifyAssignment(e, assigneeId);
  e.next();
}, "cards");

onRecordAfterUpdateSuccess((e) => {
  const before = e.record.original();
  const beforeAssignee = before ? before.get("assigneeRef") : null;
  const afterAssignee = e.record.get("assigneeRef");
  if (afterAssignee && afterAssignee !== beforeAssignee) {
    notifyAssignment(e, afterAssignee);
  }
  e.next();
}, "cards");

// ---------------------------------------------------------------------
// Overdue-card flagger
// ---------------------------------------------------------------------

const OVERDUE_CRON_ID = "team_board_overdue_flagger";
// Every minute — croner normalizes away the seconds field
// (crates/server/src/cron.rs), so this is as fine-grained as cronAdd
// gets. Fine for a demo; a real deployment would run this hourly/daily.
const OVERDUE_CRON_EXPR = "* * * * *";

cronAdd(OVERDUE_CRON_ID, OVERDUE_CRON_EXPR, () => {
  try {
    const nowIso = new Date().toISOString();
    const overdue = $app.findRecordsByFilter(
      "cards",
      "isQuery = false && overdueNotified = false && dueAt != '' && dueAt < {:now}",
      "",
      200,
      0,
      { now: nowIso },
    );
    if (overdue.length === 0) return;

    for (const card of overdue) {
      card.set("overdueNotified", true);
      $app.save(card);

      const assigneeId = card.get("assigneeRef");
      if (!assigneeId) continue;
      const assignee = $app.findRecordById("users", assigneeId);
      if (!assignee) continue;

      const notifications = $app.findCollectionByNameOrId("notifications");
      const notification = new Record(notifications, {
        userRef: assigneeId,
        teamRef: card.get("teamRef"),
        cardRef: card.id,
        message: 'Card overdue: "' + card.getString("title") + '"',
        read: false,
      });
      $app.save(notification);

      const mail = $app.newMailClient();
      mail.send({
        to: [{ address: assignee.getString("email"), name: assignee.getString("name") }],
        subject: "Overdue: " + card.getString("title"),
        text: 'Your card "' + card.getString("title") + '" is past its due date.',
        html: '<p>Your card <strong>' + card.getString("title") + "</strong> is past its due date.</p>",
      });
    }
    $app.logger().info("team-board: flagged overdue cards", "count", overdue.length);
  } catch (err) {
    $app.logger().error("team-board: overdue flagger cron failed", "error", String(err));
  }
});
