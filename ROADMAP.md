# Roadmap

Cratebase's v1 scope is intentionally PocketBase-shaped: dynamic
collections, auth, files, realtime, on SQLite or Postgres, with an admin
dashboard, one binary. What's below is what's deliberately **not** in v1,
roughly in the order it's likely to land.

## Next up

- **Plugin system.** A way to install optional server-side extensions
  without forking Cratebase — the mechanism that turns collections/rules/
  storage into a platform instead of a fixed feature set, and (eventually)
  a way for plugin authors to charge for their work. First candidate
  plugin: **feature flags** (a self-contained collection + evaluation rule
  + tiny SDK helper) — small enough to validate the plugin architecture
  before bigger ones.
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
