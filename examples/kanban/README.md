# Cratebase Kanban Example

The flagship demo: a shared, realtime, drag-and-drop Kanban board. Sign in,
drag a card from Todo into In Progress, and watch it glide into place in a
second browser tab with zero polling and zero reload — the same
register → login → authenticated CRUD → realtime path the todo example
walks, but built to actually feel like a product screen instead of a form
with a database behind it.

## Why Vite + React here, unlike the other examples

`todo/`, `realtime-chat/`, and `realtime-cursors/` are deliberately
zero-build: a single `app.js` loaded as a native ES module, with an
`importmap` in `index.html` resolving the bare specifier `"pocketbase"` to
the published package on esm.sh. That's the right call for those three —
each is a few hundred lines of vanilla DOM manipulation, and a build step
would be pure overhead.

This example is different on purpose. A polished drag-and-drop board with
live multi-column reflow, FLIP-animated repositioning, and a presence bar
is meaningfully easier to get right — and to keep readable — as a small
component tree with real state management than as hand-rolled
`document.createElement` calls. So this one has an actual `package.json`,
installs the official `pocketbase` npm package as a normal dependency
(rather than the esm.sh CDN import map), and runs on Vite + React +
TypeScript with its own dev server. `app.js`'s realtime/optimistic-update
pattern carries over unchanged in spirit — see "How it works" below — it's
just spread across hooks and components instead of one file.

## 1. Start Cratebase

From the repo root:

```bash
cargo run -p cratebase-server --bin cratebase -- serve
```

This serves the API at `http://localhost:8090` (override with
`?api=http://host:port` in the app's URL if you're running it elsewhere).

## 2. Provision the `cards` and `presence` collections

```bash
bun run examples:kanban:setup
# or directly: bash examples/kanban/setup.sh
```

This upserts an `admin@example.com` / `changeme123` superuser (override
with `ADMIN_EMAIL`/`ADMIN_PASSWORD` env vars, same as every other
example's setup.sh) and creates or updates two collections:

- **`cards`** — `title` (text), `status` (select: `todo` / `in_progress`
  / `done`), `order` (number). All five rules are
  `"@request.auth.id != \"\""` — any signed-in user can list/view/create/
  update/delete, so it behaves like a shared team board once you're
  signed in. Same deliberate simplification the todo example documents:
  no per-card ownership field, because this demo is about auth + rules +
  realtime working together, not access-control granularity.
- **`presence`** — `userId` (text, unique-indexed), `name` (text),
  `lastSeen` (autodate, updates on every write). Same rules as `cards`.
  See "Presence" below for what this is for.

Safe to re-run: if either collection already exists, `setup.sh` PATCHes
just the rules back in line and leaves the fields (and your board's
cards) alone — this is `examples/lib/setup-common.sh`'s
`ensure_collection` helper, shared by all four examples' setup scripts.

## 3. Run the dev server

```bash
cd examples/kanban
npm install
npm run dev
```

This is **not** part of `bun run examples:serve` — that script serves the
other three examples' static files from the repo root over one shared
static server, which doesn't apply here: this example has an actual build
step and needs Vite's own dev server (with its module graph, HMR, and
TypeScript transform) rather than a directory of files served as-is. Vite
prints the URL it's listening on — `http://localhost:5174/` by default,
deliberately a different port than `web/admin`'s dev server (5173) so both
can run side by side. Open that URL, register an account (email + password,
8 characters minimum, which registers and signs you in immediately), and
open the same URL in a second tab to see the two-tab realtime sync.

## How it works

- **Auth**: `useAuth` mirrors the todo example's exact calls —
  `pb.collection("users").create(...)` then `authWithPassword(...)` for
  register, `authWithPassword` alone for sign in — and subscribes to the
  SDK's `authStore.onChange` to drive the auth-screen ↔ board swap.
  `cards` and `presence` share that same client/auth store, so every
  request after sign-in automatically carries the bearer token; no manual
  header wiring anywhere in the app.
- **Cards + ordering**: each card has a `status` and a numeric `order`.
  Reordering (within a column or across columns) computes a new `order`
  as the midpoint between the two neighboring cards' `order` values
  (`orderForIndex` in `src/hooks/useCards.ts`) — a classic fractional-index
  scheme, so a drop only ever writes the one moved card, never renumbers
  its neighbors. Landing at either end of a column steps 1000 past the
  current edge instead of taking a midpoint.
- **Drag-and-drop**: plain HTML5 `draggable` + `dragstart`/`dragover`/`drop`
  — deliberately not `@dnd-kit` (which `web/admin` already depends on
  elsewhere in this repo) or any other library. Three fixed columns and
  single-card reordering is squarely inside what native DnD handles
  cleanly; a library's sensor/collision/keyboard-accessibility machinery
  would be solving problems this board doesn't have. `Column` computes the
  drop index from the dragged pointer's Y position against sibling
  card rects on every `dragover`, `Board` owns the single source of truth
  for "what's being dragged, over which column, at which index," and drop
  applies the move optimistically (instant reorder in the acting tab) then
  persists via `cards.update(id, { status, order })` — the same
  optimistic-then-reconcile-via-realtime-event pattern as the todo
  example's toggle/delete.
- **Realtime**: `useCards` calls `pb.collection("cards").subscribe("*",
  callback)` once on mount. Every `create`/`update`/`delete` event merges
  into local state by id — the acting tab's own optimistic write and the
  realtime event for that same write converge on an identical record.
  `createCard`'s optimistic insert specifically checks for an existing id
  before pushing rather than pushing unconditionally, because the
  realtime `create` event for a just-created card can arrive and get
  merged *before* the creating tab's own `await ...create()` call
  resolves — an unconditional push there raced a real duplicate-card
  render in live-testing (see "Verification status"). With the id check,
  whichever of the two arrives first wins the insert and the second is a
  no-op merge, so there's no double-render either way, and every other
  open tab picks up the change with no polling.
- **FLIP animation, not teleporting**: `src/hooks/useFlip.ts` keeps a
  registry of every rendered card's DOM node (shared across all three
  columns, since a card can move *between* columns) and, in a
  `useLayoutEffect` keyed off each card's `id:status:order`, diffs the
  previously measured bounding rect against the freshly rendered one for
  every card. Anything that moved gets its delta applied as an instant
  (no-transition) `transform`, then released on the next animation frame
  with a `transform 320ms` transition — the classic FLIP technique. This
  fires identically whether the reorder came from a local drag or a
  realtime event from another tab, so a card sliding into a new spot
  because *someone else* moved it animates exactly like a local drag does.
- **Presence — approach and why**: Cratebase's realtime layer is an SSE
  broadcast per collection; there's no server-side "who's connected"
  registry to query, so this demo approximates presence client-side with
  the `presence` collection instead of inventing a server feature. Each
  tab upserts its own record on mount (create once, remembering the id in
  `localStorage` so a reload updates the same record instead of spawning
  a new one — same create-or-update-by-remembered-id trick as the
  realtime-cursors example) and merges that record into local state
  immediately rather than waiting for the realtime round trip — a first
  version of this hook subscribed *after* the initial upsert and only
  merged incoming events, so a tab briefly read itself as offline for up
  to one heartbeat interval; live-testing caught this, see "Verification
  status" below. A background 8-second heartbeat re-upserts afterward,
  and every tab subscribes to `presence`'s realtime events so a peer
  showing up is near-instant. A 1-second client-side clock separately
  filters out any peer whose `lastSeen` has gone stale for >20s, which is
  what covers a tab that closes without a clean event (crash, killed
  process, dropped network) — `beforeunload` fires a best-effort
  `navigator.sendBeacon` DELETE for the common case, but the staleness
  filter is what makes "online" eventually correct regardless.
- **Avatars**: `src/components/Avatar.tsx` hashes the user id into a hue
  and a shape (circle / hexagon / rounded-square), rendered as inline SVG
  with two dot "eyes" — same id always produces the same avatar, no
  external asset or illustration library, no per-user data stored beyond
  the id already in `presence`.
- **Deletion**: `cards.delete(id)`, applied optimistically (removed from
  local state immediately) and reverted if the request fails.

## Verification status

**Live-verified**, driven end to end in headless Chromium against a
locally running Cratebase server (the prebuilt release binary, `serve
--http 127.0.0.1:8095`, pointed at an isolated data directory rather than
this repo's shared dev instance — a sibling agent had `cargo run` mid-edit
in this same worktree at the time), `bash examples/kanban/setup.sh`, and
`npm run dev`. Two tabs were genuinely independent sessions: the second
used a separate Puppeteer incognito browser context (its own cookie jar
and `localStorage`, not just a second tab of the same profile), navigated
with `?api=http://127.0.0.1:8095` since this test server wasn't on the
app's default port.

- `bash examples/kanban/setup.sh` — ran twice. First run created both the
  `cards` and `presence` collections; second run PATCHed the same rules
  back (`Updated` for both), confirming idempotency.
- `npm run build` (`tsc -b && vite build`) passed cleanly, both before and
  after the two fixes below.
- Registered "alice" in tab 1 and "bob" in tab 2 (independent contexts).
  Both signed in immediately into the empty three-column board.
- **Found and fixed live**: presence initially showed "0 online" in a
  freshly-signed-in tab for up to 8 seconds (the length of the heartbeat
  interval), because the hook subscribed to realtime *after* upserting
  its own presence record, missing that record's own `create` event.
  Fixed by merging the upsert's return value into state immediately
  (`src/hooks/usePresence.ts`). Re-tested after the fix: both tabs showed
  "2 online" with the correct avatars within ~1 second of sign-in, no
  heartbeat wait needed.
- Added a card ("Design the board") in tab 1's Todo column — appeared
  instantly there, and in tab 2 within 600ms with no reload (confirmed by
  reading both DOMs' card ids/titles directly, not just eyeballing).
- **Found and fixed live**: dragging that card produced two DOM cards
  with the *identical* record id in the Todo column. Root cause:
  `createCard`'s optimistic insert pushed the new record unconditionally,
  but the realtime `create` event for that same record occasionally
  resolved first and inserted it via the subscribe handler — the
  optimistic push then added a second copy of the same id instead of
  finding it already present. Fixed by checking for an existing id before
  pushing (`src/hooks/useCards.ts`). Re-tested after the fix with a fresh
  server (wiped `pb_data`, re-ran `setup.sh`, re-registered both users):
  creating a card produced exactly one DOM node.
- Dragged the card from Todo to In Progress in tab 1 (native
  `DragEvent`s dispatched with a rendering yield between `dragstart` /
  `dragover` / `drop` so React's state updates land between them, the way
  a real mouse-driven drag naturally spaces them) — moved instantly in
  tab 1, and was confirmed in tab 2 within 600ms with `status` read back
  as `in_progress` and both columns' counts updated (`todo: 0, in
  progress: 1`), with no reload.
- Deleted the card from tab 2 and confirmed it vanished from tab 1 live
  (card count went to zero in a DOM read, not just a screenshot).
- Sampled the dragged card's inline `style.transform`/`style.transition`
  every 25ms across a second drag-and-drop: `transition: transform 320ms
  cubic-bezier(0.2, 0.8, 0.2, 1)` was present starting from the first
  sample after the drop, confirming `useFlip`'s FLIP branch actually
  engages on a real move (as opposed to a silent no-op) — this checks
  that the animation *fires*, not that it *looks* good; the latter was a
  visual spot-check only (headless screenshots weren't compared against
  any baseline).

**Not verified**: `npm run preview` (the production build served
standalone) was not separately booted — only `npm run dev` was driven
live. No automated test suite was written or run; every check above was
a manual, one-off browser-driven verification, not a repeatable test.

**A pre-existing, unrelated side effect of this testing**: an early
registration attempt (before the `?api=` override was in place) briefly
hit this repo's already-running shared dev server on the default port
8090 instead of the isolated test server, creating a `users` record for
`alice@example.com` there. That server's data directory belongs to
whichever other session set it up; no schema or rule changes were made to
it, and no admin credentials for it were available to inspect or remove
that record.

