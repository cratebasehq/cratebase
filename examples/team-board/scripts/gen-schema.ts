#!/usr/bin/env bun
// Generates `schema.json` — the "schema as code" file `cratebase schema
// push schema.json` applies (see scripts/setup.sh). Written as a script
// rather than a hand-typed JSON file because every custom collection here
// has a `relation` field pointing at Cratebase's built-in `_teams` (team
// scoping) or `users` (assignee/author/uploader) collections, and
// `FieldKind::Relation::collectionId` (`crates/core/src/field.rs`) must be
// a *real* collection id, not a name — Cratebase (like PocketBase) derives
// every collection's id deterministically from its type+name
// (`scripts/lib/ids.ts`), so every id this file needs can be computed
// offline, in one pass, with no dependency-ordered "create it, read the id
// back, patch the next collection" dance (contrast
// `examples/docmind/setup.sh`, which predates this trick and does exactly
// that dance by hand).
//
// Team scoping follows the canonical pattern documented in
// `crates/server/src/teams.rs` (module doc, ~line 29): give a collection
// its own `teamRef` relation to `_teams`, then gate every rule with
//   @collection._team_members.userRef ?= @request.auth.id &&
//   @collection._team_members.teamRef ?= teamRef
// — "is the caller a member of *this row's* team", enforced in SQL, not
// application code. `_teams`/`_team_members` always exist regardless of
// `settings.teams.enabled` (that toggle only gates the server-side hook
// that auto-inserts a team's owner membership row on creation) — see
// `scripts/setup.sh` for turning it on.
import { collectionId } from "./lib/ids.js";

const TEAMS_ID = collectionId("base", "_teams");
// Unlike every other collection here, the built-in `users` collection
// does *not* get the derived id: `Collection::default_users()`
// (`crates/core/src/collection.rs`) overrides it with PocketBase's own
// historical fixed literal, `USERS_COLLECTION_ID` (`crates/core/src/lib.rs`).
// `collectionId("auth", "users")` would silently compute the wrong id.
const USERS_ID = "_pb_users_auth_";
const COLUMNS_ID = collectionId("base", "columns");
const CARDS_ID = collectionId("base", "cards");

/** "Is the signed-in user a member of the team this row belongs to" —
 * see the module doc. `teamRef` is deliberately bare (not
 * `@request.body.teamRef`): on create it resolves against the submitted
 * data, on everything else against the row itself, exactly like
 * `_teams`'s own built-in `createRule` (`ownerRef = @request.auth.id`,
 * `crates/core/src/collection.rs:939-940`). */
const MEMBER_OF_TEAM =
  '@collection._team_members.userRef ?= @request.auth.id && @collection._team_members.teamRef ?= teamRef';

const teamRelation = (extra: Record<string, unknown> = {}) => ({
  name: "teamRef",
  type: "relation",
  collectionId: TEAMS_ID,
  required: true,
  cascadeDelete: true,
  maxSelect: 1,
  ...extra,
});

const timestamps = [
  { name: "created", type: "autodate", onCreate: true },
  { name: "updated", type: "autodate", onCreate: true, onUpdate: true },
];

const collections = [
  {
    name: "columns",
    type: "base",
    fields: [
      teamRelation(),
      { name: "name", type: "text", required: true },
      { name: "order", type: "number", required: true },
      ...timestamps,
    ],
    listRule: MEMBER_OF_TEAM,
    viewRule: MEMBER_OF_TEAM,
    createRule: MEMBER_OF_TEAM,
    updateRule: MEMBER_OF_TEAM,
    deleteRule: MEMBER_OF_TEAM,
  },
  {
    name: "cards",
    type: "base",
    fields: [
      teamRelation(),
      // Not required at the DB level (`minSelect: 0`): a throwaway
      // "search query" row (`isQuery: true`, see pb_hooks/team-board.pb.js's
      // `/api/search` route) has no column. Every *real* card is required
      // to have one — enforced in `onRecordCreate` in the hooks file,
      // where it can be conditioned on `isQuery`, which a DB-level
      // `required: true` can't express.
      { name: "columnRef", type: "relation", collectionId: COLUMNS_ID, required: false, cascadeDelete: true, minSelect: 0, maxSelect: 1 },
      { name: "title", type: "text", required: true },
      { name: "description", type: "text" },
      // Server-derived by the same hook (`title + "\n\n" + description`)
      // on every create/update — the single source-field the `embedding`
      // vector field below can point at (auto-embed config takes exactly
      // one `sourceField`, not a list — see `crates/core/src/field.rs`'s
      // `EmbeddingConfig`).
      { name: "searchText", type: "text" },
      { name: "assigneeRef", type: "relation", collectionId: USERS_ID, required: false, cascadeDelete: false, minSelect: 0, maxSelect: 1 },
      { name: "labels", type: "select", values: ["bug", "feature", "chore", "urgent", "design"], maxSelect: 5 },
      { name: "order", type: "number", required: true },
      { name: "dueAt", type: "date" },
      { name: "overdueNotified", type: "bool" },
      // Marks the throwaway row `/api/search` creates to get a query
      // vector out of the real embedding pipeline (see that hook for
      // why this can't just be a one-off computation). Excluded from
      // every rule below (`isQuery = false`) so it's invisible to every
      // client, including in realtime — the same filter that gates a
      // list request also gates which realtime events a subscriber
      // receives.
      { name: "isQuery", type: "bool" },
      {
        name: "embedding",
        type: "vector",
        dimensions: 64,
        // "echo" is the deterministic, network-free provider
        // (`crates/server/src/embeddings.rs`) — always available
        // regardless of `EMBEDDINGS_BASE_URL`, so semantic search works
        // fully offline. Swap to `{"provider": "openai", "model":
        // "text-embedding-3-small"}` (and unset nothing else) for real
        // embeddings once `EMBEDDINGS_BASE_URL`/`EMBEDDINGS_API_KEY` are set.
        embedding: { provider: "echo", model: "", sourceField: "searchText" },
      },
      ...timestamps,
    ],
    listRule: `${MEMBER_OF_TEAM} && isQuery = false`,
    viewRule: `${MEMBER_OF_TEAM} && isQuery = false`,
    createRule: MEMBER_OF_TEAM,
    updateRule: MEMBER_OF_TEAM,
    deleteRule: MEMBER_OF_TEAM,
  },
  {
    name: "comments",
    type: "base",
    fields: [
      teamRelation(),
      { name: "cardRef", type: "relation", collectionId: CARDS_ID, required: true, cascadeDelete: true, maxSelect: 1 },
      { name: "authorRef", type: "relation", collectionId: USERS_ID, required: true, cascadeDelete: false, maxSelect: 1 },
      { name: "body", type: "text", required: true },
      ...timestamps,
    ],
    listRule: MEMBER_OF_TEAM,
    viewRule: MEMBER_OF_TEAM,
    createRule: `${MEMBER_OF_TEAM} && authorRef = @request.auth.id`,
    updateRule: "authorRef = @request.auth.id",
    deleteRule: "authorRef = @request.auth.id",
  },
  {
    name: "attachments",
    type: "base",
    fields: [
      teamRelation(),
      { name: "cardRef", type: "relation", collectionId: CARDS_ID, required: true, cascadeDelete: true, maxSelect: 1 },
      { name: "uploaderRef", type: "relation", collectionId: USERS_ID, required: true, cascadeDelete: false, maxSelect: 1 },
      { name: "file", type: "file", maxSelect: 1, maxSize: 10485760 },
      { name: "created", type: "autodate", onCreate: true },
    ],
    listRule: MEMBER_OF_TEAM,
    viewRule: MEMBER_OF_TEAM,
    createRule: `${MEMBER_OF_TEAM} && uploaderRef = @request.auth.id`,
    updateRule: "uploaderRef = @request.auth.id",
    deleteRule: "uploaderRef = @request.auth.id",
  },
  {
    name: "notifications",
    type: "base",
    fields: [
      teamRelation(),
      { name: "userRef", type: "relation", collectionId: USERS_ID, required: true, cascadeDelete: true, maxSelect: 1 },
      { name: "cardRef", type: "relation", collectionId: CARDS_ID, required: true, cascadeDelete: true, maxSelect: 1 },
      { name: "message", type: "text", required: true },
      { name: "read", type: "bool" },
      { name: "created", type: "autodate", onCreate: true },
    ],
    // Written exclusively by pb_hooks/team-board.pb.js via `e.app.save()`
    // (bypasses API rules entirely, like every `$app` write) — no end
    // user ever POSTs one directly, hence `createRule: null`.
    listRule: "userRef = @request.auth.id",
    viewRule: "userRef = @request.auth.id",
    createRule: null,
    updateRule: "userRef = @request.auth.id",
    deleteRule: "userRef = @request.auth.id",
  },
  {
    name: "presence",
    type: "base",
    fields: [
      teamRelation(),
      { name: "userRef", type: "relation", collectionId: USERS_ID, required: true, cascadeDelete: true, maxSelect: 1 },
      { name: "name", type: "text", required: true },
      { name: "lastSeenAt", type: "date", required: true },
    ],
    indexes: ["CREATE UNIQUE INDEX `idx_presence_team_user` ON `presence` (`teamRef`, `userRef`)"],
    listRule: MEMBER_OF_TEAM,
    viewRule: MEMBER_OF_TEAM,
    createRule: `${MEMBER_OF_TEAM} && userRef = @request.auth.id`,
    updateRule: `${MEMBER_OF_TEAM} && userRef = @request.auth.id`,
    deleteRule: "userRef = @request.auth.id",
  },
];

const outPath = new URL("../schema.json", import.meta.url);
await Bun.write(outPath, JSON.stringify({ collections }, null, 2) + "\n");
console.log(`wrote ${collections.length} collection(s) to ${outPath.pathname}`);
