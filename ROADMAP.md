# Roadmap

Cratebase's v1 scope is intentionally PocketBase-shaped: dynamic
collections, auth, files, realtime, on SQLite or Postgres, with an admin
dashboard, one binary. What's below is what's deliberately **not** in v1,
roughly in the order it's likely to land.

## Next up

- **Plugin system.** Shipped as of `crates/server/src/plugin.rs`: a
  `Plugin` trait with two extension points — `routes()` to mount extra
  HTTP endpoints under `/api/plugins/<name>`, and `scheduled_tasks()` to
  run fixed-interval background jobs (Cratebase's answer to PocketBase's
  cron, minus calendar expressions — see the trait doc comment for why).
  This is a compile-time Rust trait, not a dynamically loaded/scripted
  plugin format: "installing a plugin" means implementing `Plugin` in
  `crates/server/src/plugins/`, registering it in `plugins::registry()`,
  and shipping your own binary. `plugins/example.rs` is a working
  reference (a `/stats` route + a 5-minute logging job) to copy from.
  **Not yet built on this foundation, but now unblocked:**
  - **Cron jobs as a plugin** — a `Plugin` whose `scheduled_tasks()` reads
    job definitions from a `_cron_jobs` collection instead of being
    hardcoded, with a run-history table and a dashboard "Jobs" tab.
  - **Team management as a plugin** — multiple admins with roles is a
    real data-model change (today there is one `_admins` table, no
    roles); once record-lifecycle hooks exist on `Plugin` this can enforce
    role checks without touching the core auth crate.
  - **Feature flags** — a self-contained collection + evaluation rule +
    tiny SDK helper, still the smallest candidate to validate a plugin
    that also needs its own collection/schema, not just routes+jobs.
  - Record lifecycle hooks (`on_create`/`on_update`/`on_delete`) are not
    on the trait yet — add them when the first plugin actually needs one,
    rather than speculatively.
- **Background jobs / queues.** Durable job processing, `pg_boss`-style
  when running on Postgres (SQLite deployments would need a different
  backend — this needs design work, not just wiring `pg_boss` in).
- **View collections.** The `Collection::View` type and `view_query` field
  already exist in the schema, but nothing executes the backing SQL yet —
  `list_records`/`get_record` assume a real physical table. Needs: safe
  view-query validation, read-only enforcement, and dashboard UI.
- **Relation dot-notation in filters** (`author.name = "..."`) — currently
  filters only see the record's own columns.
- **"Any of" filter operators** (PocketBase's `?=`, `?!=`, ...) for
  multi-value fields.

## Later

- OAuth2 providers for auth collections (currently email+password only).
- Email verification / password reset flows (currently no email sending at
  all — auth collections have no `sendEmail` hook).
- Realtime beyond a single node (Postgres `LISTEN/NOTIFY` or a queue as the
  fan-out layer, so `RealtimeHub` isn't in-process-only).
- File field constraints in the dashboard UI (`mimeTypes`, `maxSize` are
  already schema fields but have no editor).
- Multi-file append/remove semantics on update (today, uploading new files
  for a field replaces the whole value; PocketBase's `field+`/`field-`
  suffixes are not implemented).
- Admin audit log / activity feed.
- Per-collection rate limiting.

## Explicitly out of scope for now

- A visual query builder for filters (the filter language is meant to be
  hand-written; a builder is a dashboard feature, not a core one).
- Multi-tenant / workspace-scoped superusers (one superuser table, full
  access to everything — same as PocketBase).
