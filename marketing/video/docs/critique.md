# Critique log

Format per round: what was reviewed, scores 1-10 on the brief's seven
axes, the 3 biggest problems with timestamps, what got fixed, what's
still open. Round continues until every score is 8+ (course step 11);
this file is the honest record of not being there yet.

# v2 (art-direction rework, 64 s, 120 BPM)

Sheets per round come from `./critique.sh <render> <strip_start> <tag>`:
`out/critique/<tag>-contact.png` (2 fps, 6 across; tile r,c = (6r+c)/2 s),
`<tag>-strip.png` (12 consecutive frames at the fastest action) and
`<tag>-phone.png` (1 fps at 360 px wide). Scores are 1–10 on hook, phone
readability, motion quality, variety, composition, brand accuracy, sound
sync. Every round below was actually looked at, not assumed.

## v2 · Round 1 — animatic (12 fps, full 64 s, synthesized score muxed)

Strip taken at 36.2 s (search re-ranking on every keystroke — the fastest
state change in the film).

| Axis | Score | Why |
|---|---|---|
| Hook | 7 | Mega words on the beat + fog cut + crate hit land hard, but 5.5–7.0 s is nearly static (only one line rising under the wordmark). |
| Phone readability | 6 | Statements, terminal, rules, auth card and inbox all read at 360 px. Superpowers panel details (search rows, board cards, notification rows) are ~30–36 px and blur out; the 2×2 wall is texture only. |
| Motion quality | 7 | Springs, camera racks and the auth morph feel expensive. Search rows cross through each other mid-swap (transparent rows, keystrokes every 16th so springs never settle); CTA ends on a near-static hold. |
| Variety | 8 | Every chapter changes ground, technique and scale; a new state at least every beat in Superpowers. |
| Composition | 7 | Full-bleed everywhere, strong focal points; map and proof cells leave some slack, CTA right-bottom quadrant empty. |
| Brand accuracy | 9 | Real palette, crate geometry from iso.ts, real product captures and fonts; v0.5 features labelled. |
| Sound sync | 6 | Score renders from the same cue sheet (164 cues) and structure matches the picture (hits at 4.0/56.0, drop at 48), but the mix has not been checked against the picture onset-by-onset yet. |

### 3 worst problems
1. **Dead holds** — 60.0–64.0 s CTA barely moves (6 % push only); 5.5–7.0 s hook near-static.
2. **Superpowers detail too small on phone** — 36–44 s panel text 29–36 px.
3. **Text overlapping during swaps** — 36.25–37.0 s search rows cross with transparent backgrounds.

### Fixes
1. CTA: a row of seven feature chips builds one per beat 60.5–63.5 s, each with a pop; hook: constant slow dolly on the crate stage 4.0–7.5 s with the wordmark drifting at a different rate (parallax).
2. Search rows 170 px (58 px titles), board cards 36 px text, notification rows 40 px, map list/pins enlarged.
3. Search rows get a solid ground and the rising row is drawn on top; keystrokes move to 8th notes (36.25–37.5 s) so each re-rank settles before the next.


## v2 · Round 2 — full pass (30 fps, 64 s, score muxed) + first 9:16 stills

Strip at 43.9 s (the solo → 2×2 wall transition, the busiest cut).
Also reviewed: 28 vertical (1080×1920) stills across every chapter, and an
objective sound-sync check (`node audio/sync-check.mjs`: energy-onset
detection on the SFX stem vs. the 163 non-swell cues).

| Axis | Score | Why |
|---|---|---|
| Hook | 8 | Mega words on beats 0–3, fog cut, crate lands on the 4.0 hit; the reveal now dollies with parallax instead of holding. |
| Phone readability | 8 | Every focal detail (rule, `{{user.name}}`, OTP digits, search rows, board cards, badge, 10.47×) reads at 360 px; the 2×2 wall is deliberately texture under a 170 px statement. |
| Motion quality | 8 | Search re-ranks settle on 8ths with solid rows (no overlapping text); CTA builds a chip per beat to the last frame. One continuity pop found at 44.0 (see below). |
| Variety | 9 | New technique per chapter; a state change every beat in Superpowers. |
| Composition | 8 | Full-bleed throughout; proof recomposed (number + full-width bars); CTA fills the lower-right with the chip row. |
| Brand accuracy | 9 | Unchanged from round 1; benchmark label corrected to "record list reads" (the `search` workload is paged listing, not FTS). |
| Sound sync | 8 | 120/163 cues have an SFX onset within 10 ms, 130 within 40 ms. The rest were checked by peak level: present but masked by a preceding tail (hit reverb, sub under the count-up) or centred by design (whooshes/risers). Key clicks were drifting (linear spread, ±30 ms jitter) — now one click per picture reveal step. Scored from analysis, not listening: nobody auditioned the mix. |

### 3 worst problems
1. **44.0 s notification rows vanish** when the panel joins the wall (its sub-timeline restarted) — a continuity pop.
2. **Mastering used loudnorm's dynamic mode** (true-peak constraint forced it), which can pump the mix.
3. **9:16 only reviewed as stills** — data chapter framing, 10.47× clipping, CTA chips off-frame and PocketBase code overflow were found and fixed from stills; the vertical cut still needs a full-render round.

### Fixes
1. The wall continues the solo's state: read rows stay on screen and re-arriving notifications light their row and badge up again.
2. Linear mastering: measured static gain to −14 LUFS, then a 4× oversampled look-ahead limiter → −14.2 LUFS, −1.7 dBTP.
3. Portrait-specific ROIs (names → types rack, 250 px rule ROI, narrower record columns), tabular-width probe for 10.47×, line-broken PocketBase snippet, lockup raised. Full 9:16 render is round 3.

## v2 · Round 3 — final 60 fps masters (16:9 and 9:16) + delivery encodes

Reviewed: strips at 3.85 s (crate drop, the fastest motion) and 27.9 s
(auth card exit), the 16:9 contact/phone sheets, the 9:16 contact and a
phone-size sheet of the vertical delivery, and the teaser contact sheet.

First look at the 60 fps `--sub 2` master found **ghosting**: two averaged
subframes turned every fast move (crate fall, card exit, typing reveals,
code-card fling) into two distinct copies instead of blur. It read as
cheap, so the masters were re-rendered at crisp 60 fps (sub 1). Also fixed
here: the `DATABASE_URL=` line blinked out on every beat at 52–54 s (the
wipe clipped the whole line); now only the value rolls.

| Axis | 16:9 | 9:16 | Why |
|---|---|---|---|
| Hook | 8 | 8 | Four mega words in the first 2 s, fog cut, crate lands on the 4.0 hit. |
| Phone readability | 8 | 8 | Every focal detail reads at 360 px wide. 9:16's data chapter frames narrower ROIs and racks between them to keep glyphs large. |
| Motion quality | 9 | 8 | Crisp 60 fps, no ghosting, no overlapping swaps, no dead holds. 9:16 inherits a few 16:9-tuned camera moves that feel slightly fast on the narrow frame. |
| Variety | 9 | 9 | Unchanged. |
| Composition | 8 | 8 | Full-bleed throughout. 9:16 data/email framings leave large dimmed plane areas above the focus: legible, but the least dense frames in the vertical cut. |
| Brand accuracy | 9 | 9 | Unchanged; all on-screen claims verified (README, CHANGELOG 0.4.0, benchmarks/README.md 2026-09-04, SDK source, feature branch for v0.5 labels). |
| Sound sync | 8 | 8 | Same analysis as round 2 on the final mix: 120/163 cues within 10 ms, the rest verified present. Final mix −14.2 LUFS integrated, −1.7 dBTP, linear. Not auditioned by a human. |

**Every score ≥ 8 — gate passed.** Residual notes for a next pass: tighten
9:16 data/email framings; have someone actually listen to the mix (score
taste is unverified by ears).

---

# v1 history (superseded animatic, 66 s)

## Round 1 — `out/animatic.mp4` (8fps, 66s, silent, no `--sub` blur)

Reviewed: `out/contact-full.png` (12x11 tile, fps=2, full 66s), a
12-frame strip around t=36.5-38.0s (the Search shot — chosen as the
fastest nominal "action" in the busiest chapter), `out/phone.png` (fps=1,
360px wide, 8x9 tile).

### Scores

| Axis | Score | Why |
|---|---|---|
| Hook (first 2s) | 7 | Kinetic type wipe + crate assembly reads clean, asymmetric, no gradient/centered tell — but the crate is small relative to how "huge" the brief's kinetic type promise implies; could be punchier. |
| Readability at phone size | 4 | Headlines and the Proof chapter's benchmark numbers read perfectly at 360px. Everything else doesn't: real UI screenshot text (schema fields, rule editor, JSON) is illegible once scaled down, and the whole Superpowers chapter's recreated panels are a few near-invisible dots on empty dark backgrounds. |
| Motion quality | 4 | Springs measure correctly (see `lib/motion.test.mjs`) and container morphs are smooth. But the strip test caught a real dead shot: Search (36.0-38.5s) is a **static screenshot with zero motion for 2.5s** — exactly the "dead beat with nothing happening" the course's own critique prompt says to hunt for. |
| Variety | 5 | Data/Auth/Email chapters have strong variety from real captured screens. Superpowers — the chapter the shot list calls the busiest, "a new thing every 2-4s" — is the least varied: five different recreated panels that all look like "a dot on a dark rectangle." |
| Composition | 6 | The one-morphing-container device works and holds together; hurt by the same empty-panel problem and by real-screenshot crops that leave visible dead whitespace inside the panel (e.g. the API-docs shot around t=15s). |
| Brand accuracy | 8 | Real palette/type/screenshots throughout; crate mark geometry is the actual `iso.ts` projection math, not redrawn; orange used only as accent, never UI chrome, matching the brand rule. |
| Sound sync | 1 | No audio pass yet — the film is currently silent. Scored low rather than "N/A" per the brief's numeric scale; this is expected at this gate, not a surprise. |

**Not at 8+ on five of seven axes.** Expected for a first animatic pass,
logged honestly rather than rounded up.

### 3 biggest problems

1. **Search (36.0-38.5s) is a literal static image, no motion at all.**
   Confirmed by the strip test: 12 consecutive frames from 36.5-38.0s are
   pixel-identical. The shot list called for "result rows re-rank live,"
   but the implementation just swaps in one screenshot and holds it.
2. **The four v0.5.0 recreations (Nearby/Presence/Notifications/Channels,
   38.5-50.0s) are visually empty** — a handful of small dots or a single
   short line of text on an otherwise bare dark panel. At phone size
   (`out/phone.png`, same time range) they're close to invisible. This is
   the busiest chapter by shot list intent and the emptiest by what's
   actually on screen.
3. **TOTP (25.5-28.0s) is an unfinished-looking placeholder** — a plain
   white square and a few empty boxes, no real visual design. Weakest
   single shot in the film on every axis at once.

### Fixed this round

- **Search (36.0-38.5s)**: replaced the static screenshot with a small
  live re-rank recreation grounded in the real seeded `places` names
  (Kopi Kenangan Senopati, Warung Tekko, Blok M Co-working Loft, Pasar
  Santa Vinyl Corner — the same records `capture/seed.sh` creates): four
  rows whose y-position is on its own `track()`, swapping order once
  partway through the shot. Real data, real motion — a screenshot alone
  can never show reordering, so this is a case where a grounded
  recreation genuinely serves the shot better than a capture could.
- **Nearby/Presence/Notifications/Channels**: added a faint ground grid,
  real place-name labels on the map pins (same four seeded places),
  cursor name labels sized up, a second/third notification row, and a
  fuller channel bubble — aimed at problem #2 without inventing anything
  not grounded in the real API shapes already cited in `shotlist.md`.

### Fix verification (same round, re-rendered as `out/animatic-r2.mp4`)

Re-ran the strip test at the same t=36.5-38.0s window and a new 6x4 tile
over 39.0-51.0s (the rest of Superpowers):

- Search: the 12-frame strip now shows four named rows (the real seeded
  places) visibly changing order partway through — no longer identical
  frames. Motion confirmed, not just code-reviewed.
- Nearby/Presence/Notifications/Channels: pins and cursors now carry real
  labels (place names + distance, "Nico"/"Amara", real comment text,
  named chat lines), each panel has a one-line code caption
  (`cb.rpc(...)`, `usePresence(...)`, `cb.notifications.list()`,
  `cb.channel(...)`) tying it back to the real API, and a faint ground
  grid gives the panel a floor instead of bare dark space. Visibly denser
  at both full size and in a fresh `phone-r2.png` — no longer close to
  invisible at 360px, though still the least visually rich chapter in
  the film relative to the real-screenshot chapters.

This was **not** a full fresh round 2 (no new contact-sheet-wide hunt for
problems, no rescoring against all seven axes) — just verification that
the two targeted fixes landed. A real round 2 is still owed before this
goes near a final render.

### Still open (next round)

- TOTP (problem #3) — not touched this round; needs a real design pass,
  not just more content.
- Screenshot crop framing — several real captures still show more
  whitespace than signal at 1920x1080 scale, and shrink to illegible at
  phone size. Needs either a tighter `object-position`/zoom per shot or
  a cropped re-capture, not a global fix.
- Hook could be punchier (score 7, not yet 8).
- No audio pass — sound sync can't be scored honestly above 1 until
  there's a score to sync to.
- This is round 1 of the course's minimum 3. Re-render the animatic
  after the fixes above and do round 2 before touching the full-quality
  render.
