//! Teams (`_teams`) and their memberships (`_team_members`):
//! application-level multi-user workspaces, shaped like a Slack
//! workspace — an ordinary end-user feature built entirely on the
//! generic Records API, *not* a way to reach the admin dashboard.
//! Membership in a team says nothing about superuser access; every rule
//! below is evaluated against `@request.auth.id` on whatever `type:
//! "auth"` collection the caller authenticated against (normally
//! `users`), the same as any other record rule.
//!
//! # Shape
//!
//! * `_teams`: `name` (text), `ownerRef` (relation → `users`, the
//!   creator).
//! * `_team_members`: `teamRef` (relation → `_teams`, cascade-delete),
//!   `userRef` (relation → `users`, cascade-delete), `role` (text, e.g.
//!   `"owner"` / `"member"`).
//!
//! Both collections are registered by
//! `cratebase_core::Collection::default_system_collections` with their
//! rules attached — see that function's own comments for the exact
//! rule strings. This module only supplies the one behaviour a rule
//! cannot express: creating a team can never itself satisfy
//! `_team_members`'s `createRule` (there is no owner row yet to prove
//! membership against), so [`bind_hooks`] reactively inserts the
//! bootstrap owner row after a `_teams` row is created, in the same
//! database transaction, using the `ownerRef` the create rule already
//! forced to equal the caller's own id.
//!
//! # Scoping a third-party collection to a team
//!
//! This is the actual point of `_teams`/`_team_members`: making an
//! **application-defined** collection visible only to the members of
//! one team. The naive rule
//!
//! ```text
//! teamRef.id = @request.auth.id
//! ```
//!
//! is wrong — that compares a relation's target id against the caller's
//! *user* id, i.e. per-user scoping, not per-team. The correct pattern
//! joins into `_team_members` with `@collection._team_members` (the
//! same relation-traversal construct `_team_members`'s own rules use to
//! check the caller against a sibling row) and asks two questions of
//! the joined row: is it the caller's own membership, and is it a
//! membership in *this record's* team:
//!
//! ```text
//! @collection._team_members.userRef ?= @request.auth.id &&
//! @collection._team_members.teamRef ?= teamRef
//! ```
//!
//! Concretely, for an app collection `docs` with its own `teamRef`
//! relation field pointing at `_teams`, set `listRule`/`viewRule` (and
//! `updateRule`/`deleteRule`, if every member should be able to edit) to
//! exactly that expression. `docs.teamRef` on the right-hand side of the
//! second comparison resolves against `docs` (the collection the rule
//! belongs to), not the joined `_team_members` row — `@collection.X.a op
//! b` compares the joined side's `a` against `b` resolved on the base
//! record, exactly like `records.test.ts`'s existing
//! `@collection.${MEMBERS}.team ?= id` fixture for a `_teams`-shaped
//! collection scoping itself to its own `id`. Verified live (see
//! `TeamsMemberships`'s task report): two members of the same team both
//! read a `docs` row carrying their team's id; a third, unrelated user
//! gets an empty list and a 404 on direct `view`.
//!
//! # Why the bootstrap insert is a raw [`cratebase_db::records::create`]
//! call, not a second pass through the Records API
//!
//! Going through `routes::records::create` again would re-run
//! `_team_members`'s `createRule` — which, as above, no caller can ever
//! satisfy for a team's very first member — and would re-fire every
//! hook tagged to `_team_members`, including realtime broadcast, for a
//! row that is really just part of finishing the `_teams` create. Same
//! reasoning as `crate::cron_jobs`'s and `crate::webhooks`'s
//! status-write-backs: this is system-internal bookkeeping, not a
//! second user-initiated write.

use cratebase_core::Record;
use serde_json::Value;

use crate::app::{App, TxApp};
use crate::events::RecordEvent;
use crate::hooks::Handler;

const COLLECTION: &str = "_teams";
const MEMBERS_COLLECTION: &str = "_team_members";

/// Bind the reactive hook. Called once from
/// [`crate::app::App::bootstrap`], tagged to `_teams` so it never fires
/// for any other collection's writes.
///
/// Bound at a negative priority (lower runs first, PocketBase's default
/// is 0) rather than the default: `crate::webhooks::bind_hooks` binds
/// its own `on_record_after_create_success` handler *untagged* — it has
/// to watch every collection's writes — and that handler returns `Ok(())`
/// directly without calling `e.next()`, terminating the chain right
/// there for whichever collection's create just ran (verified live,
/// otherwise silent: no error, no panic — the rest of the chain simply
/// never executes). Any handler registered after it in priority order,
/// tagged or not, would never run for *any* collection unless it sorts
/// ahead of that handler instead.
pub fn bind_hooks(app: &App) {
    app.hooks().on_record_after_create_success.bind(
        Handler::new(|e: &mut RecordEvent| {
            // Built synchronously, from data owned outright (a cloned
            // `TxApp`, an owned `Record`): `RecordEvent` itself is `Send`
            // but not `Sync` (its hook chain's finalizer is a boxed
            // `FnOnce`), so a reference to it can never be held across
            // an `.await` inside a handler future that must itself be
            // `Send`.
            let task = prepare_owner_membership(e);
            Box::pin(async move {
                if let Some((app, team_id, mut member)) = task {
                    if let Err(err) =
                        cratebase_db::records::create(&app, &app.db().collections, &mut member)
                            .await
                    {
                        tracing::warn!(error = %err, team = %team_id, "failed to insert bootstrap team owner membership");
                    }
                }
                Ok(())
            })
        })
        .with_tags([COLLECTION])
        .with_priority(-10),
    );
}

/// Build the `_team_members` owner row for the `_teams` record that was
/// just created, ready to insert in the same transaction ([`TxApp`]
/// carries the in-flight transaction handle). `ownerRef` is trusted as
/// the creator's id without re-deriving it from an auth context:
/// `_teams`'s `createRule` (`@request.auth.id != '' && ownerRef =
/// @request.auth.id`) already forced the two to match before this
/// record could exist at all.
///
/// Returns `None` (and logs) rather than failing the triggering create:
/// a database written some other way (a direct SQL insert, a migration)
/// may produce a `_teams` row with a blank or dangling `ownerRef`, and
/// that row existing without a member is a lesser problem than the
/// create itself failing after the row is already committed-to by the
/// rest of the hook chain.
fn prepare_owner_membership(e: &RecordEvent) -> Option<(TxApp, String, Record)> {
    let members = e.app.db().collections.get(MEMBERS_COLLECTION);
    let team_id = e.record.id().to_string();
    let Some(members) = members else {
        tracing::warn!("{MEMBERS_COLLECTION} missing, cannot bootstrap team owner");
        return None;
    };
    let owner_ref = e
        .record
        .get("ownerRef")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if owner_ref.is_empty() {
        tracing::warn!(team = %team_id, "_teams row created with blank ownerRef, no owner membership inserted");
        return None;
    }

    let mut member = Record::new(members);
    member.set("teamRef", Value::String(team_id.clone()));
    member.set("userRef", Value::String(owner_ref));
    member.set("role", Value::String("owner".to_string()));
    Some((e.app.clone(), team_id, member))
}
