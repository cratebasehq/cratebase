# Shot list — Cratebase launch film

66s · 1920×1080 master (16:9), reflowed to 1080×1920 (9:16) and a 15s
teaser cut from the same timeline · 120 BPM, 4/4 → 1 beat = 0.5s, 1 bar =
2s, 33 bars total.

**One deliberate hard cut in the whole film** (Superpowers → Proof, 50.0s).
Everything else is a continuous morph of a single container — the crate
mark unfolding into a terminal, then a dashboard panel that reshapes
through every feature — per `docs/style_guide.md`'s "one morphing
container" section. This is the one place we spend the film's boldness on
cut grammar, in line with the design direction's "spend your boldness in
one place" rather than varying the cut style shot to shot.

**Chapter boundaries refined onto bar lines** from the brief's approximate
beat sheet (which says "refine in shotlist.md"): Auth/Email/Superpowers
shifted by ≤1s each so every chapter change lands on a downbeat instead of
mid-bar. Total duration (66s) and beat order are unchanged.

| Chapter | Brief mark | Refined | Bars |
|---|---|---|---|
| Hook | 0–4s | 0.0–4.0s | 1–2 |
| One binary | 4–10s | 4.0–10.0s | 2–5 |
| Data | 10–20s | 10.0–20.0s | 5–10 |
| Auth | 20–29s | 20.0–28.0s | 10–14 |
| Email | 29–37s | 28.0–36.0s | 14–18 |
| Superpowers | 37–50s | 36.0–50.0s | 18–25 |
| Proof | 50–58s | 50.0–58.0s | 25–29 |
| CTA | 58–66s | 58.0–66.0s | 29–33 |

Legend: **REAL** = captured screenshot/DOM of the actual v0.4.0 dashboard
or SDK output. **RECREATE** = v0.5.0-only feature (channels/presence/
notifications, unreleased — see `feat/notifications-realtime` in this
repo for the real API/SDK shapes it's built from), animated faithfully
from real code, never imagined UI.

---

## 1. Hook — 0.0–4.0s (bars 1–2)

**1.1 — 0.0–2.0s.** Dark ground (`#0a1622`). Kinetic type, asymmetric
(lower-left third, never centered): "Stop rebuilding" wipes in left→right
on the Heavy spring at t=0.0 (downbeat, bar 1); "your backend." wipes in
0.5s later (beat 2), stacked below, same left edge. No accent color yet.

> **On-screen text:** "Stop rebuilding your backend."

**1.2 — 2.0–4.0s.** The negative space under the second line resolves into
the crate mark: three faces (top/left/right, real geometry from
`iso.ts`/`CrateBox.astro`) spring in staggered 0.03s apart (Heavy,
k=150/d=34), then the brace lines. Crate settles bottom-right of the
type, camera push-in (0.94→1.0, Default spring) completes exactly at
4.0s as the panel begins its morph into a terminal (no cut — see Ch. 2).

**SFX:** low kick at 0.0 and 2.0 (bar downbeats). Three soft wood-block
ticks as the crate's three faces land (~2.0–2.4s). No music bed yet
under 1.1 — silence-then-hit makes the first kick read as the hook.

---

## 2. One binary — 4.0–10.0s (bars 2–5)

**2.1 — 4.0–6.0s.** REAL. Crate's top face flattens/widens (Default
spring, `track()` on width+height+radius) into a terminal title bar; the
terminal body drops open below it. Monospace text types at 8th-note
intervals:
```
$ curl -fsSL https://cratebase.dev/install.sh | sh
cratebase 0.4.0 installed to ~/.local/bin/cratebase
```
(verbatim from `site/src/pages/index.astro`'s own terminal transcript.)

**2.2 — 6.0–8.0s.** REAL. New line types `$ cratebase dev --dir demo`,
then the real startup banner prints (per README.md's documented output):
API + dashboard URLs, the printed-once superuser line, the types-file
path, the mail-inbox URL. A short caption sweeps in bottom-left at 8.0s:

> **On-screen caption:** "One binary."

**2.3 — 8.0–10.0s.** REAL. Terminal window itself morphs (Default spring
on w/h/radius) directly into the captured dashboard's collections list —
the "dashboard blooms out of the terminal" beat from the brief, done as
one continuous container resize, not a cut. Push-in settles at 10.0s
exactly on the downbeat that opens Data.

**SFX:** synthesized key-clack per character (filtered noise burst,
~60ms), a rising two-note chime on the banner printing, a soft whoosh on
the terminal→dashboard morph.

---

## 3. Data — 10.0–20.0s (bars 5–10)

Demo schema is the brief's own: a `places` collection (`name`: text,
`description`: text, full-text searchable, `location`: geoPoint, `owner`:
relation → users) plus `users` and `posts` — captured live against
`cratebase dev` in `marketing/video/capture/`.

**3.1 — 10.0–13.0s.** REAL. Section title "Data" wipes in top-left at
10.0s, exits by 13.0s. Dashboard panel morphs into the collection-editor
screen; cursor clicks "+ Add field" and types each of the four `places`
fields in turn, field-type dropdown opening/closing for `location`
(geoPoint) and `owner` (relation).

**3.2 — 13.0–16.5s.** REAL. Panel morphs into a split composition (code
pane + dashboard, side by side in 16:9 / stacked in 9:16): the real
`cratebase typegen` output —
```ts
interface PlacesRecord {
  id: string; name: string; description: string;
  location: GeoPoint; owner: string;
}
```
— and a real SDK call, `cb.collection("places").list()`, resolving to a
populated records table.

**3.3 — 16.5–20.0s.** REAL. Panel morphs into the rule editor. Cursor
types the filter expression into the `updateRule`/`deleteRule` field:
```
owner = @request.auth.id
```
A one-line confirmation state (real dashboard behavior: the rule saves
inline) lands on the 20.0s downbeat, which is also the morph's completion
into Auth — no cut.

**SFX:** typing clicks per field name; a rising confirm chime when the
rule saves; sub-bass pulse on every downbeat (10.0, 12.0, 14.0, 16.0,
18.0, 20.0) — the pulse becomes audible from this chapter on, tempo
otherwise unchanged.

---

## 4. Auth — 20.0–28.0s (bars 10–14)

**4.1 — 20.0–23.0s.** REAL. Section title "Auth" wipes in. Panel morphs
into the auth-providers settings screen. Provider icons (the real 0.4.0
set — Google, Apple, GitHub, Microsoft, Discord, GitLab, Facebook, X,
LinkedIn, Slack, Twitch, Spotify, per `CHANGELOG.md` 0.4.0) snap into a
grid one at a time on the Chrome-snap spring (k=260/d=20 — the one place
outside the crate/type where a tiny overshoot is allowed), each arrival
on a 16th-note (~0.19s apart), quietest to loudest visually as the grid
fills.

**4.2 — 23.0–25.5s.** REAL. One sign-in card, one continuous morph across
three real auth methods via `track()` (never three separate cards):
magic-link (email field + "we'll email you a link") → OTP (6-digit code
boxes) → TOTP 2FA (QR code + 6-digit input, matching the real 0.4.0
feature). Content swaps behind the clip-path text-exit/enter rule, never
a fade.

**4.3 — 25.5–28.0s.** REAL. Card settles on TOTP as the shot holds long
enough to read it, then the panel begins its Email morph on the 28.0s
downbeat.

**SFX:** a short percussive "snap" per provider icon landing, pitched up
slightly across the run (a rising xylophone-like run of 12 short notes);
a soft whoosh on each of the two card-content morphs.

---

## 5. Email — 28.0–36.0s (bars 14–18)

**5.1 — 28.0–31.0s.** REAL. Section title "Email" wipes in. Panel morphs
into the real visual template editor (React Email Editor–based). Cursor
selects a text block and inserts a `{{user.name}}` variable chip — real
feature, real syntax.

**5.2 — 31.0–33.5s.** REAL/code overlay. A code pane overlays showing the
real send call:
```ts
await cb.mails.send({
  template: "welcome",
  to: user.email,
});
```

> **On-screen caption:** "Send it from your frontend."

**5.3 — 33.5–36.0s.** REAL. A no-code trigger row highlights (record-event
trigger firing), then the panel morphs into the real dev mail inbox
(`cratebase dev`'s built-in inbox) showing the sent email rendered with
`{{user.name}}` resolved to a placeholder name. Morph into Superpowers
completes on the 36.0s downbeat.

**SFX:** a paper/mail "whoosh-thunk" on send; a single UI click on the
variable-chip insert.

---

## 6. Superpowers — 36.0–50.0s (bars 18–25)

The busiest chapter: five real capabilities in 14s, one per ~2.5–3s. No
big section-title banner here — each micro-feature gets a small mono
label, bottom-left, naming the real thing on screen (concrete labels, not
a generic "Superpowers" title card, which never appears in the film
itself — it's only our internal chapter name).

**6.1 — 36.0–38.5s.** REAL (FTS already shipped, `main`). Label: "Search."
Typing in a search box against the `places`/`posts` full-text index;
result rows re-rank live, each row's y-position on its own spring via
`track()` so reordering reads as motion, not a redraw.

**6.2 — 38.5–41.0s.** REAL. Label: "Nearby." A code pane shows a real
custom-RPC call, `cb.rpc("nearest_places", { lat, lng })`; a map-style
card (drawn in the isometric brand language, not a literal map skin — no
third-party map brand in a Cratebase film) drops pins in distance order.
Caption: "PostGIS-accelerated."

**6.3 — 41.0–44.5s.** RECREATE, from real `usePresence`/
`useRecordPresence` hook shapes (`sdk/js/react/src/usePresence.ts`,
`useRecordPresence.ts`) and the repo's own `realtime-cursors` example.
Label: "Presence." Two labeled cursor dots move across a shared board on
seeded-noise paths (`mulberry32`, never `Math.random`) — never identical
runs, always deterministic.

**6.4 — 44.5–47.5s.** RECREATE, from the real `cb.notifications` API
(`list`/`markRead`/`unreadCount`, `sdk/js/client/src/index.ts`). Label:
"Notifications." A bell icon's badge count increments as a row arrives,
then resolves to zero on a "mark all read" click.

**6.5 — 47.5–50.0s.** RECREATE, from the real `cb.channel(name)` API and
the repo's own `realtime-chat` example. Label: "Channels." One chat
bubble arrives in a small panel, held just long enough to read before the
hard cut.

**SFX:** five distinct short percussive timbres, one per micro-feature,
so the ear tracks each cut even without reading; tempo feel doubles to
8th-note hats under this chapter only (still 120 BPM underneath — a
density change, not a tempo change).

---

## 7. Proof — 50.0–58.0s (bars 25–29) — hard cut in

**7.1 — 50.0–54.0s.** A single sub-kick lands on the cut. Real benchmark
data only, from `benchmarks/README.md`'s 2026-09-04 idle-host run: a bar
chart for `search` at concurrency 50 — Cratebase 49,986 req/s vs
PocketBase 4,774 req/s. Bars grow on the Default spring; the ratio figure
counts up as a genuine digit-roll on tabular mono figures (a `track()`d
number, not a text trick) to **10.47x**.

> **On-screen caption:** "10.47x faster reads at concurrency 50. See
> benchmarks/README.md."

**7.2 — 54.0–56.0s.** A toggle morphs the same records table between two
small badges, "SQLite" and "Postgres," with identical results under each
— same API, same filter syntax, same rules either way (real claim).

**7.3 — 56.0–58.0s.** A small panel shows the same SDK call working
unchanged; caption states the real compatibility claim plainly.

> **On-screen caption:** "PocketBase-wire-compatible."

**SFX:** the one sub-kick at 50.0; a synthesized "zipper" tick synced to
each digit change during the count-up; otherwise the sparsest chapter in
the film — proof needs quiet to be read twice.

---

## 8. CTA — 58.0–66.0s (bars 29–33)

**8.1 — 58.0–61.0s.** The benchmark panel flattens back down (Default
spring, reverse of Ch. 1's assembly) into the crate silhouette — a visual
rhyme with the opening frame. Stage clears to dark ground.

**8.2 — 61.0–64.0s.** Final headline wipes in, asymmetric, crate beside
it (not behind/centered):

> **On-screen text:** "The backend you actually ship with."

**8.3 — 64.0–66.0s.** Three real links, stacked, left-aligned, each on
its own line (never mid-dot joined):
```
cratebase.dev
GitHub
Open in Codespaces
```
Camera is fully static for this whole shot — the second and last static
hold in the film (the first was the Proof chart), used deliberately so
stillness reads as the ending. **65.0s is the poster frame**: headline,
crate and links all settled, nothing mid-motion.

**SFX:** the score resolves to a single sustained chord under 8.2; one
soft low thud on the final downbeat (66.0s, bar 33). Open question for
the audio pass: whether the chord's natural decay needs ~1s of tail past
66.0s (in which case the deliverable's audio track is briefly longer than
the picture, faded under the last frame) or whether to voice it so it
resolves cleanly by 66.0s exactly — decide once the score exists and the
render contract's exact `DUR` is locked.

---

## Open items carried to the next gate (asset capture)

- Exact copy for dropdown/field-type labels in 3.1 depends on what the
  real dashboard renders for a `geoPoint`/`relation` field type — capture
  first, don't guess the label text.
- 4.1's 12-provider grid needs the real icon set as shipped in the
  dashboard (`web/admin`), not redrawn logos — confirm during capture
  which are inline SVGs vs sprite assets.
- 6.2's isometric map-card is a new brand-consistent element (no existing
  illustration for "map"); sketch it as a still before animating, matches
  the Stills gate.
