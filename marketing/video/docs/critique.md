# Critique log

Format per round: what was reviewed, scores 1-10 on the brief's seven
axes, the 3 biggest problems with timestamps, what got fixed, what's
still open. Round continues until every score is 8+ (course step 11);
this file is the honest record of not being there yet.

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
