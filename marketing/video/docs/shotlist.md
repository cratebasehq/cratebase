# Shot list — Cratebase launch film (v2)

64 s = 32 bars at 120 BPM (beat 0.5 s, bar 2 s). 1920×1080 master, 1080×1920
reflow, 15 s teaser cut from the same timeline. Times are absolute seconds;
`b` = beat. Density target: a meaningful change at least every beat in
Superpowers, at least every 1.5 s everywhere else.

Legend — **REAL**: pixels from the real v0.4.0 dashboard (`capture/shots/hd-*.png`,
DPR 4, with measured `boxes.json`). **REBUILT**: a real shipped example's UI
rebuilt as DOM from its source so it can morph and scale crisply
(`examples/team-board/src/components/*.tsx`, same copy, tokens and fonts).
**RECREATE**: a feature without a capturable UI, drawn from its real API
shape. On-screen claims are quoted from README.md / CHANGELOG.md /
benchmarks/README.md / site index.

| Chapter | Time | Bars | Ground |
|---|---|---|---|
| Hook | 0–8 | 1–4 | navy → fog → navy |
| One binary | 8–14 | 5–7 | terminal black |
| Data | 14–22 | 8–11 | dashboard black |
| Auth | 22–30 | 12–15 | paper/fog |
| Email | 30–36 | 16–18 | dashboard black |
| Superpowers | 36–48 | 19–24 | team-board ink |
| Proof | 48–56 | 25–28 | navy |
| CTA | 56–64 | 29–32 | navy |

---

## 1. Hook (0–8)

**1.1 · 0.0–2.0 — the list you keep rebuilding.** Navy. Four mega words, one
per beat, condensed Bricolage ~440 px, left-aligned to a 96 px margin:
"Auth." (0.0) "Files." (0.5) "Realtime." (1.0) "Email." (1.5). Each rises
out of a hard mask on the heavy spring and shoves the previous word up and
out of the top edge (one `track()` on the stack's y). One element per
beat, nothing else on screen. SFX: kick + minor stab per word.

**1.2 · 2.0–4.0 — "Stop rebuilding your backend."** Hard cut to fog on
2.0. Harbor-ink display type 250 px: "Stop rebuilding" rises at 2.0,
"your backend." at 2.5 (8th-note push-up on each line). 3.5–4.0: both
lines are shoved down out of frame as a shadow falls from above. SFX:
riser from 2.0, reverse swell into 4.0.

**1.3 · 4.0–6.0 — crate reveal (hero hit).** Navy. The crate mark
(`iso.ts` geometry, real face colours) drops from above and lands exactly
on 4.0 at ~78% frame height, right half; impact dip on the whole stage.
Braces draw on 4.5. "Cratebase" wordmark (display 250 px) rises on 5.0 in
the left half. SFX: THE hit (sub drop + noise crack + long tail), then pad.

**1.4 · 6.0–8.0 — into the crate.** "A backend in one crate." (site hero,
statement 120 px) rises under the wordmark on 6.0. On 7.0 the camera
pushes into the crate's top face until orange fills the frame (7.5) and
the orange face darkens into the terminal ground at 8.0. SFX: whoosh.

## 2. One binary (8–14) — terminal, REAL transcript

Full-bleed terminal plane (tilted ~12°), Red Hat Mono 56 px.
- **8.0–9.25** `$ curl -fsSL https://cratebase.dev/install.sh | sh` types in
  16th-note bursts (click per burst). 9.25 output
  `cratebase 0.4.0 installed to ~/.local/bin/cratebase` (site transcript).
- **9.5–10.0** `$ cratebase dev` types.
- **10.0–11.0** the real `cratebase dev` banner prints one line per 16th
  (verbatim from a real run: API, Dashboard, Superuser, Types, Mail inbox).
- **10.0** statement "One binary." (Display 240 px) rises in the left band;
  the terminal plane slides right to make room. 11.0 second line, from
  the README tagline: "Collections, auth, files, realtime." (104 px).
- **12.0** camera racks to the `Dashboard:` line; highlight ring; cursor
  click 12.5; the dashboard (REAL `hd-overview`) swings in from depth and
  fills the frame by 13.0; camera glides to its Collections table (users,
  places, posts) by 14.0.

## 3. Data (14–22) — REAL dashboard captures

- **14.0–16.0 `hd-places-schema`, Fields.** Camera frames the four field
  rows (ROI from boxes). Rows unmask one per beat (14.0 name, 14.5
  description, 15.0 location, 15.5 owner). Callouts pop on the type
  selectors: "Text", "Geo point", "Relation → users". Statement (left
  band): "Model your data."
- **16.0–18.0 same capture, API rules.** Camera racks down to the rules.
  16.5 the Update rule text types in (mask reveal per 16th):
  `owner = @request.auth.id`; push-in until the rule is ~70 px. Callout
  17.25: "Only the owner can edit." Statement: "Rules are filter
  expressions."
- **18.0–20.0 `hd-places-records`.** Whip to records; the four seeded rows
  unmask on 8ths; camera pans name → location → owner. Callout on
  location: "Geo point".
- **20.0–22.0 code card** floats over the records on its own Z layer (soft
  shadow): `const places = await cb.collection("places").list({ sort:
  "-created" })` + the generated `PlacesRecord` type. Statement: "Typed SDK,
  generated types."

## 4. Auth (22–30) — REBUILT team-board sign-in card (paper)

The real `AuthScreen.tsx` card, rebuilt at scale; camera pushes in so tab
labels render ≥ 48 px. A cursor drives every step. Left band: the method
name as kinetic type, swapped by mask on each bar.
- **22.0–24.0 "Magic link."** Card arrives from depth; cursor clicks
  "Magic link" (22.5, indicator stretches); email types
  `alice@example.com` (23.0); click "Send magic link" (23.5) → the real
  sent copy: "If alice@example.com has an account, a sign-in link is on its
  way…".
- **24.0–26.0 "Email code."** Click "Email code" (24.0); "Send code"
  (24.5) → Code field; six digits type on 8ths (25.0–25.75) with
  `tracking-widest`.
- **26.0–28.0 "Authenticator app."** RECREATE (TOTP has a real API —
  `cb.auth.signIn.totp({ mfaId, code })`, RFC 6238, 6 digits, 30 s — but no
  example UI): the card morphs (height/content on `track()`) into a
  6-box code step with a 30 s ring; digits land on 8ths; check at 27.5.
- **28.0–30.0 "12 OAuth2 presets."** The card's real "Continue with …"
  buttons (Google, GitHub, Apple, Microsoft, Discord, GitLab, Facebook, X,
  LinkedIn, Slack, Twitch, Spotify — the 12 `KnownProvider` presets) burst
  out into a frame-filling 4×3 wall, one per 16th, camera pulling back.
  Sub-line: "plus any OpenID Connect provider."

## 5. Email (30–36)

- **30.0–32.0 REAL `hd-email-editor`.** Camera starts wide on the HTML
  editor, pushes in to line 1: `Welcome, {{user.name}}`. A crate ring
  locks around `{{user.name}}` (30.75) and a crisp chip lifts off the
  plane toward camera. Statement: "Templates with variables."
- **32.0–34.0 code card:** `await cb.mails.send({ template: "welcome", to:
  "amara@cratebase.dev", data: { user: { name: "Amara" } } })` types;
  33.5 the card is flung right (whoosh). Statement: "Send it from your
  frontend."
- **34.0–36.0 REAL `hd-mail-inbox`.** The "Welcome to Acme" row slides in
  (34.0); camera pushes into the reading pane until "Welcome, Amara" is
  ~150 px (35.0). Callout: `{{user.name}}` → "Amara".

## 6. Superpowers (36–48) — the densest chapter, team-board ink

Recreations in team-board's real dark tokens and fonts, fed by the film's
real seeded data (places/posts) and team-board's real seed (board columns,
cards, users Alice Chen, Bob Diaz, Carol Nkemelu).
- **36.0–38.0 Search.** Full-width search field (text 96 px) types
  "coffee" one letter per 8th; results (4 places + 2 posts) re-rank on
  every keystroke by prefix match — "c" matches co-working/crate/calls,
  "cof" narrows to the coffee place — each row's y on its own `track()`,
  matched terms highlighted. Chip: `cb.collection("places").list({ search:
  "coffee" })`. Statement: "Full-text search."
- **38.0–40.0 Nearby.** Full-bleed dark street map; "You" pulse; radius
  sweep 38.0→38.75; pins drop in distance order on 8ths (real seeded
  lat/lng, distances by haversine: Kopi Kenangan Senopati, Blok M
  Co-working Loft, Pasar Santa Vinyl Corner, Warung Tekko); ranked list
  with metres. Chip: `sort: "geoDistance(location.lon, location.lat, …)"`
  (real filter function, verified against a live server).
- **40.0–42.0 Presence.** The team-board board (Backlog / In Progress /
  Review / Done, real card titles); four labelled cursors (Alice, Bob,
  Carol, you) fly on seeded paths; avatar stack counts 1→4 on beats; Bob
  drags "Dark mode for the dashboard" to Done (41.0). Chip:
  `usePresence(cb, "board:acme", { cursor })`.
- **42.0–44.0 Notifications.** Bell (huge) with badge counting 1→6 on 8ths,
  rows cascading ("Carol assigned you…"); 43.5 cursor clicks "Mark all
  read" → badge snaps to 0. Chip: `cb.notifications.unreadCount()`.
- **44.0–46.0 All at once.** Camera pulls back: the four panels are one 2×2
  wall, all live (a new query types, pins pulse, cursors move, badge
  climbs). Statement: "Realtime, built in."
- **46.0–48.0** the wall tilts into perspective and the camera flies over
  it; 47.5 hard cut on the bar to navy.

## 7. Proof (48–56) — navy, real numbers only

- **48.0–50.0** Sub hit. Mega "10.47×" counts up on tabular digits; two
  full-width bars grow: Cratebase 49,986 req/s (crate) vs PocketBase 4,774
  req/s. Lines: "Full-text search, 50 concurrent clients" /
  "benchmarks/README.md, run of 2026-09-04".
- **50.0–52.0** three more cells stack in on beats: 9.72× (search as a
  signed-in user, c100), 8.53× (wide search, c100), 1.81× (create, c100).
  Statement: "Faster in 22 of 24 benchmark cells."
- **52.0–54.0** `DATABASE_URL=sqlite://…` flips to `postgres://…` on each
  beat while the result list beside it stays identical. Statement: "SQLite
  or Postgres. Zero code changes."
- **54.0–56.0** `import PocketBase from "pocketbase"` + "180/181 SDK
  conformance tests pass." Statement: "PocketBase-wire-compatible."

## 8. CTA (56–64)

- **56.0** hit: the crate lands again (rhyme with 4.0), wordmark rises 56.5,
  "A backend in one crate." 57.0.
- **58.0** install command types in a pill, one burst per 16th.
- **59.0 / 59.5** `cratebase.dev` and `github.com/cratebasehq/cratebase`
  rise; "MIT licensed. Self-hosted. One binary." 60.0.
- **60.0–64.0** slow push-in on the lockup, final chord; nothing else moves.
  Poster = the last frame.

## Teaser (15 s)

Master segments on beat boundaries: 0.0–4.0 (hook) · 36.0–40.0 (search +
nearby) · 22.5–25.0 (auth) · 48.0–50.0 (proof) · 56.0–58.5 (CTA) =
4 + 4 + 2.5 + 2 + 2.5 = 15 s. Audio is the master mix cut at the same
points with 12 ms crossfades (every segment starts on the grid).
