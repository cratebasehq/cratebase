# team-board

[![Open in GitHub Codespaces](https://github.com/codespaces/badge.svg)](https://codespaces.new/cratebasehq/cratebase)

The flagship example: a small-team project workspace — teams, a
per-team Kanban board with realtime drag-and-drop, presence, card
detail (assignee, labels, comments, file attachments), and an
AI-powered semantic search box — built to exercise most of what
Cratebase can do in one real app, not six disconnected demos.

**Screenshots**: not included — this was built and verified from a
terminal with no browser available (see "Verification status" below),
so no screenshot was captured rather than fabricated. Board, card
detail, and auth-screen screenshots would go here
(`docs/screenshot-{board,card-detail,auth}.png`) once someone runs this
with a browser attached.

## 1. Run it

From the repo root, in this directory:

```bash
cd examples/team-board
bun run dev
```

This one command:

1. Builds `cratebase-server` if `target/debug/cratebase` doesn't exist
   yet (`cargo build -p cratebase-server`, `CARGO_BUILD_JOBS=3`).
2. Starts it (`cratebase serve --dev --dir ./pb_data`) with a fixed
   `CB_SETUP_TOKEN` and `CB_TYPEGEN_OUT` pointed at
   `src/cratebase-types.d.ts`.
3. Runs `scripts/setup.sh`: creates the superuser via `POST /api/setup`,
   turns on `settings.teams.enabled`, generates and pushes `schema.json`
   (`cratebase schema push`), and regenerates TypeScript types.
4. Seeds demo data (`scripts/seed.ts`) — skipped if it's already there.
5. Starts Vite.

Open the printed Vite URL (`http://localhost:5175/` by default) and
sign in with `alice@example.com` / `password123` (or `bob@example.com`,
same password, same team; `carol@example.com` is on a second team, for
exercising cross-team access denial — see "API rules" below). The
Cratebase dashboard is at `http://localhost:8090/_/`
(`admin@example.com` / `changeme123`).

Running in a Codespace? `postAttachCommand` in `.devcontainer/devcontainer.json`
already runs `bun run dev` for you — the workspace should come up with
the board already running; open the forwarded `5175` port.

## 2. Design

A shipping-crate/manifest visual vocabulary, tied directly to the
Cratebase name, rather than a generic SaaS-card template or the
cream/terracotta look generated UIs default to:

- **Color**: warm paper (`#F6F4EF`) / ink (`#16181D`) surfaces, one bold
  accent — crate orange (`#E8622C`) — spent sparingly (primary actions,
  active states, the semantic-search affordance). Manifest blue and moss
  green cover secondary/informational and positive states.
- **Type**: Archivo (display/headings), Inter (body/UI), IBM Plex Mono
  for literal data — card reference tags (`CB-1a2b`), timestamps — never
  as decoration.
- **Structure over shadow**: flat surfaces with 1px borders doing the
  structural work; the one shadow in the system (`shadow-lift`) is a
  hard-edged, offset "lifted crate" read that only appears on a dragged
  card, not ambient decoration under every panel.
- **Layout**: split auth screen (dark brand panel + form, not a
  centered card), a right-side slide-over for card detail (keeps the
  board visible/contextual instead of a blocking modal), horizontal
  board columns.

## 3. What's here

- **Auth**: email+password sign-up/sign-in, an OTP ("email code") tab
  (`cb.auth.otp.request`/`signIn.otp` — the code lands in the dev mail
  inbox), and OAuth buttons that only render when
  `GET /api/collections/users/auth-methods` actually lists a configured
  provider (none are configured in this example by default).
- **Teams**: `_teams`/`_team_members` (Cratebase's built-in team
  collections, turned on via `settings.teams.enabled` in
  `scripts/setup.sh`) with a team switcher. Every custom collection has
  its own `teamRef` relation and is scoped by the canonical rule pattern
  from `crates/server/src/teams.rs`:
  `@collection._team_members.userRef ?= @request.auth.id && @collection._team_members.teamRef ?= teamRef`.
- **Board**: columns + cards, drag-and-drop reordering (kanban's
  fractional-index scheme, `src/hooks/useCards.ts`), realtime across
  tabs, hand-rolled per-team presence avatars (`src/hooks/useBoardPresence.ts`
  — see its module doc for why not `@cratebase/react`'s generic
  `usePresence`).
- **Card detail**: inline-editable title/description, assignee (relation
  + expand), label chips, a realtime comment thread, file attachments
  with image thumbnails (`?thumb=100x100`).
- **Semantic search**: cards auto-embed a derived `searchText` field
  (title + description) with Cratebase's offline "echo" embedder — no
  `EMBEDDINGS_BASE_URL` needed. The search box (`src/hooks/useSearch.ts`)
  creates a throwaway `isQuery: true` card to get a query vector out of
  the real embedding pipeline, ranks with `cb.vector.nearestTo`, then
  deletes it — invisible to every client (including its own creator),
  in both listings and realtime, because `cards`' rules exclude
  `isQuery = true` rows.
- **`pb_hooks/team-board.pb.js`**: derives `searchText`, notifies +
  emails a card's assignee on (re)assignment, and a `cronAdd` job that
  flags overdue cards once a minute.
- **Types**: `cratebase typegen` → `src/cratebase-types.d.ts` (not
  committed — see `.gitignore`), consumed via
  `createCratebaseHooks<Schema>()`. `_teams`/`_team_members` are system
  collections excluded from typegen, so they're typed by hand in
  `src/lib/teams.ts`.
- **Schema-as-code**: `scripts/gen-schema.ts` generates `schema.json`
  with every `relation` field's `collectionId` computed *offline*
  (`scripts/lib/ids.ts`) — Cratebase derives every collection's id
  deterministically from its type+name, so this needs no
  create-then-patch dance across dependent collections. `cratebase
  schema push schema.json` applies it.
- **Seed data**: `seed.json` (`{collection: [records]}`) + `scripts/seed.ts`,
  which resolves `"$alias"` references between records and writes them
  through `@cratebase/client` as superuser — structured so a future
  `cratebase seed` command could read the same file directly.

## Environment variables

Same conventions as every other example (`CRATEBASE_URL`,
`SUPERUSER_EMAIL`/`SUPERUSER_PASSWORD`), plus:

| Variable | Default | Meaning |
|---|---|---|
| `CRATEBASE_HOST` / `CRATEBASE_PORT` | `127.0.0.1` / `8090` | Where `cratebase serve` listens |
| `CB_SETUP_TOKEN` | `team-board-dev-setup-token` | First-run install token for `POST /api/setup` |
| `CB_DATA_DIR` | `./pb_data` | Data directory (siblings `./pb_hooks`) |
| `CRATEBASE_BIN` | `../../target/debug/cratebase` if built, else `cratebase` on `PATH` | Server binary to run |

## Verification status

See the PR/commit description for what was actually run — server boot,
setup, seed, `bunx tsc --noEmit`, `bun run build`, sign-in/cross-team
API checks via curl, a realtime event, semantic search, the assignment
hook creating a notification, and mail landing in `/api/dev/mails`.
