# Plugin dashboard panels — design

## Problem

Cratebase's `Plugin` trait (`crates/server/src/plugin.rs`) lets a downstream
project extend the server — one-time setup, extra HTTP routes, scheduled
background jobs — without forking this repo, including from a binary that
only depends on `cratebase-server` as a library. The three built-in
plugins (`cron_jobs`, `feature_flags`, `queue`) each provision their own
system collection and are usable today through the fully generic
collections admin UI with zero extra code.

`cron_jobs` additionally got a hand-rolled, polished dashboard page
(`web/admin/src/components/settings/cron-jobs-page.tsx`) this session:
colored status badges, an inline enabled toggle, a friendlier drawer.
`queue` and `feature_flags` were about to get the same treatment the same
way — hardcoded React, compiled into `web/admin`, embedded into the
binary at build time.

That doesn't scale to third-party plugins. A plugin author using
`cratebase-server` as a library gets the generic collections UI for free,
but no path to a *polished* panel without forking `web/admin` and
rebuilding the whole dashboard. This design closes that gap: a plugin
declares a small, purely declarative panel description; the *existing*
admin dashboard renders it generically. No per-plugin React required,
built-in or third-party.

## Scope

**In scope:** a declarative *table* view (columns with display hints, a
small fixed action vocabulary, an optional exact-count summary strip) for
a plugin-owned collection. Create/edit reuses the existing generic
per-field-type record drawer unmodified.

**Out of scope (explicitly deferred, not silently dropped):**
- Custom create/edit forms beyond the generic drawer. No current plugin
  needs it (cron_jobs/queue/feature_flags are all plain text/bool/
  select/number/date/json fields, which the generic drawer already
  handles well after this session's field-editor UX work).
- A plugin shipping its own bundled UI code (module federation or
  similar). Real trust-boundary problem (arbitrary JS in the admin's
  authenticated browser session) and a build/versioning problem between
  the dashboard bundle and plugin bundles — a v2+ decision needing its
  own security design, not folded into this one.

## Data model

New optional `Plugin` trait method, default `None`:

```rust
fn dashboard_panel(&self) -> Option<PanelDescriptor> { None }
```

```rust
pub struct PanelDescriptor {
    pub name: String,                    // stable id; also the panel's route segment
    pub label: String,                   // sidebar/tab label
    pub icon: PanelIcon,                 // fixed enum -> lucide icon, frontend-side lookup
    pub collection: String,              // which collection this panel lists
    pub default_sort: Option<String>,    // e.g. "-availableAt"
    pub columns: Vec<PanelColumn>,
    pub actions: Vec<PanelAction>,
    pub summary: Option<PanelSummary>,
}

pub enum PanelIcon { Clock, ListTree, Flag, Layers /* extend as needed; unknown value from an
    older/newer server version falls back to a generic icon, never fails to render */ }

pub struct PanelColumn {
    pub field: String,
    pub label: Option<String>,           // defaults to title-cased field name
    pub hint: ColumnHint,
}

pub enum ColumnHint {
    Text,
    Code,                                 // monospace
    Badge { colors: BTreeMap<String, BadgeColor> }, // value -> color, for select/status fields
    BoolToggle,                           // reuses the existing generic inline-editable bool cell
    Date,
    Json,
    Fraction { of: String },              // e.g. attempts shown as "N / maxAttempts"
}

/// Small fixed palette, not a freeform color/hex — matches the existing
/// chip-color conventions already in `record-value-cell.tsx` (light/dark
/// pairs are a frontend-side lookup keyed by this enum, same as
/// `PanelIcon`'s icon lookup).
pub enum BadgeColor { Gray, Blue, Green, Amber, Red, Purple }

pub struct PanelAction {
    pub id: String,
    pub label: String,
    pub kind: ActionKind,
    pub visible_when: Option<FieldEquals>, // e.g. only show "Retry" when status == "failed"
}

/// `field`'s current value on the row must equal `value` (JSON deep-eq)
/// for the action to render. No other comparator (not-equal, contains,
/// etc.) — the two known uses (queue's Retry, and any future action
/// gated on a status-like field) are both a plain equality check; add a
/// comparator only when a real second shape shows up.
pub struct FieldEquals { pub field: String, pub value: serde_json::Value }

pub enum ActionKind {
    Delete,
    PatchFields(BTreeMap<String, PatchValue>),
}

pub enum PatchValue { Literal(serde_json::Value), Now }

pub struct PanelSummary {
    pub group_by_field: String,          // e.g. "status" -> exact counts per distinct value
}
```

`PluginRegistry` auto-exposes `GET /api/plugins/_dashboard` (admin-only,
`RequireAdmin`) returning `Vec<PanelDescriptor>` for every registered
plugin whose `dashboard_panel()` returns `Some`. No per-plugin route
wiring — register the plugin, the panel appears.

**Reserved name.** `PluginRegistry::register()` panics at startup
(`.expect`-style, matching the existing "invalid static config panics"
convention e.g. the rate-limit governor config) if `plugin.name() ==
"_dashboard"` — fails loud at boot instead of risking a silent route
collision with the registry's own auto-exposed endpoint.

## Exact summary counts

`PanelSummary.group_by_field` must reflect the *whole* collection, not
whatever page the panel's table happens to have loaded — a queue with 500
jobs showing "3 failed" from page 1 while the real count is different is
a wrong number, not an approximation. New generic endpoint:

```
GET /api/collections/{name}/records/_group_counts?field=status
```

Admin-only (mirrors every other collection-management endpoint), runs
`SELECT {field}, COUNT(*) FROM {table} GROUP BY {field}` through the same
backend abstraction `crates/db/src/records.rs` already uses. Generically
useful beyond plugin panels (any collection with a select/status-like
field benefits), not panel-specific plumbing.

## Frontend rendering

- On Settings mount, fetch `/api/plugins/_dashboard` once.
- `settings-tabs.tsx` becomes dynamic: the two collection-less tabs
  (Logs, Backups) stay hardcoded; one additional tab renders per fetched
  descriptor.
- `router.tsx` gets one dynamic route `/settings/plugin/$name` instead of
  one named route per plugin.
- New `PluginPanelPage` component: given a `PanelDescriptor`, calls
  `useRecords(descriptor.collection, ...)` for the table and the new
  `_group_counts` endpoint for the summary strip (skipped entirely if
  `summary` is absent). Column rendering reuses existing logic
  (`record-value-cell.tsx`'s badge-color/date/json rendering,
  `inline-cell.tsx`'s bool toggle) rather than reimplementing it. Actions
  render as buttons; `Delete` calls `useRecordMutations(...).remove`,
  `PatchFields` calls `.update` with the resolved literal/`Now` values.
  "Edit" is never a descriptor action — clicking a row (same convention
  as the generic collections grid) opens the existing generic record
  drawer.

## Migration

`cron_jobs`, `queue`, `feature_flags` each implement `dashboard_panel()`
with the descriptors sketched in the brainstorm (cron_jobs: name/text,
schedule/code, enabled/boolToggle, lastRunAt/date, lastStatus/badge;
queue: queue/text, status/badge, attempts/fraction, availableAt/date,
lastError/text, Retry action gated on status=failed, summary by status;
feature_flags: key/code, enabled/boolToggle, rule/code). Delete
`cron-jobs-page.tsx`/`cron-job-drawer.tsx` in favor of the generic
renderer.

## Testing

- Backend: `GET /api/plugins/_dashboard` — 401 unauthenticated, 200 with
  the 3 expected descriptors for a superuser. `GET .../_group_counts` —
  exact counts across multiple pages worth of rows (seed >1 page, assert
  the total matches, not just page-1 tallies). A `PluginRegistry` test
  asserting `register()` panics for a plugin named `_dashboard`.
- Frontend: `bunx tsc -b --noEmit`, plus a **live browser pass on all
  three panels** (not just typecheck, since this migration deletes a
  page that was already live-verified once) — toggle persists via a
  follow-up API call, Retry actually flips a failed job back to pending,
  delete actually removes the row, and the summary strip's counts are
  verified against a seeded row count that exceeds one page.
