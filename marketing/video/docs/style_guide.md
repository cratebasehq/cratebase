# Style guide — Cratebase launch film (v2 art direction)

v1 (the animatic in git history, commit a667b5e) failed review for one core
reason: **everything was small**. UI panels filled ~30% of a dark navy
frame, whole dashboards shrank until their text was noise, and the hook
type was a caption. v2 keeps v1's brand accuracy (palette, fonts, the real
isometric crate) and its engine, and changes the camera and the scale.

## The idea in one line

**The camera is inside the product.** Nothing sits in the middle of an
empty frame. Real UI runs edge to edge, a virtual camera travels across it
and pushes in until the one detail the story is about (a rule expression,
a `{{user.name}}` variable, a digit in an OTP box) is the size of a
headline, and giant type owns whatever the UI doesn't.

Named look: Linear / Vercel / Raycast launch films. We take their grammar
(full-bleed product, confident camera, big type, hard rhythm), never their
footage.

## Palette (from `site/src/styles/tokens.css`, plus the two real app palettes)

| Role | Hex | Where |
|---|---|---|
| Harbor night (film ground) | `#0a1622` | Hook, terminal, proof, CTA grounds |
| Harbor panel | `#101f30` | Code cards floating over UI |
| Ink on dark | `#eef1f4` | Display type on dark |
| Secondary on dark | `#93a4b6` | Mono annotations, source lines |
| Fog (light ground) | `#eef1f4` | Hook "Stop rebuilding" beat, Auth chapter |
| Harbor ink (on light) | `#0e2238` | Display type on fog |
| Crate | `#ef7d3e` | The crate mark, highlight rings on UI details, the one bar that wins, cursor. Never a UI button colour. |
| Crate faces | top `#ef7d3e`, left `color-mix(crate 86%, harbor)` ≈ `#d26f3a`, right `color-mix(crate 70%, harbor)` ≈ `#b05f37` | Crate mark only (`IsoFrame.astro`) |
| Dashboard (real, captured) | near-black `#0a0a0a` chrome | Never recoloured; it is what the product looks like |
| team-board app (real example, for Auth + Superpowers) | ink `#16181D` / `#1B1E25` / `#2B2F38`, paper `#F6F4EF` / `#FFFFFF` / `#EFEBE2`, slate `#C9C2B4`, crate `#E8622C` (dark `#F07440`), manifest `#3A5B8C` (light `#6C8FC7`) | `examples/team-board/tailwind.config.js` |

Grounds alternate on purpose so the cut rhythm is visible even as a
thumbnail: navy (hook) → fog (hook line 2) → navy (crate) → black (terminal,
dashboard) → paper/fog (auth) → black (email) → ink (superpowers) → navy
(proof, CTA). No gradients anywhere, no glows on UI chrome.

## Type

- **Display**: Bricolage Grotesque (variable, full axes: `wght`, `wdth`
  75–100, `opsz`). Hook words are set **condensed** (`wdth` 75, `wght`
  800) so a single word can be 380–460 px tall and still fit 16:9 width.
  Chapter statements `wdth` 88, `wght` 760. Tracking −0.035em at display
  sizes. Line-height 0.84 for stacked hook words, 0.95 for statements.
- **Mono**: Red Hat Mono for film annotations and code cards.
- **Product faces** stay the product's own: IBM Plex Sans + JetBrains Mono
  for dashboard recreations (from `web/admin/src/index.css`), Archivo +
  Inter + IBM Plex Mono for team-board recreations. All fonts vendored in
  `assets/fonts/` so renders are offline and deterministic.
- Sentence case. No eyebrow labels, no all-caps, no `→` suffixes, no
  mid-dot meta strings, no single accented word in a headline.

### Type scale (16:9 master at 1080 p; 9:16 in brackets at 1080 w)

| Level | Size | Use |
|---|---|---|
| Mega | 400–460 px [300] | Hook words, the 10.47× number |
| Display | 220–260 px [170] | "Stop rebuilding / your backend.", wordmark |
| Statement | 120–150 px [104] | One per chapter beat, e.g. "Rules are filter expressions." |
| Callout | 64–76 px [60] | Labels anchored to a UI detail |
| Code | 44–52 px [40] | Floating code cards, terminal |
| Floor | **52 px** [34] | Nothing the viewer must read is smaller. That is ≥ 9.7 px on a 360 px-wide phone. |

UI captures are exempt from the floor as texture, but **the focal detail
of every UI shot is framed so its glyphs render ≥ 52 px** (dashboard 14 px
CSS text × camera zoom ≥ 3.7).

## Framing rules

1. **Full frame, always.** No shot shows a panel floating in empty
   ground. UI planes are larger than the frame; type is set to touch or
   nearly touch the frame edges.
2. **One focal point per beat**, and it is the largest or brightest thing
   on screen. Everything else is context: out of focus by scale, cropped,
   or dimmed by depth.
3. **Scale contrast inside every frame**: one mega/display element next to
   small precise mono annotation.
4. **Type never covers the focal UI detail.** In 16:9 statements live in a
   band (left third or lower third) and the camera frames the UI in the
   remaining region; in 9:16 the band is the top third.
5. No corner labels, timecodes, counters, frame borders, safe-area marks.

## Camera language

The camera is a real 3D camera over flat planes, written as
`matrix3d` so screen positions of UI details can be computed exactly
(`lib/cam.js`) and callouts can be pinned to them.

- **Push-in to detail** (primary move): camera frames a region of interest
  (ROI) measured from the capture's `boxes.json`; `track()` on centre x/y
  and on log-zoom so zooms feel constant-rate, not accelerating.
- **Rack**: travelling from one ROI to the next within the same capture,
  with a short overshoot-free Default spring. Never a cut inside a capture.
- **Tilt**: planes live at 6–16° of X/Y rotation with 1600–2200 px
  perspective; they flatten toward 0° when the viewer needs to read, and
  tilt away on exits. Depth reads from perspective and soft shadow, never
  from blur-for-its-own-sake.
- **Layered parallax**: callouts and code cards sit on their own Z layers
  (projected, not CSS-3D, so text is always crisp) and drift at a
  different rate than the UI plane.
- **Impact dip**: on the two hits (crate reveal, CTA) the whole stage
  dips 10–16 px on a heavy spring. The only "shake" in the film.
- **Speed**: moves settle in 0.35–0.6 s; a new move starts every beat or
  two. Holds longer than 1.5 s must contain meaningful motion (typing, a
  counter, a cursor, a slow push).

## Motion (closed-form springs, `lib/motion.js`)

| Preset | k | d | Use |
|---|---|---|---|
| snappy | 320 | 28 | Cursor, tab indicator, callout pop |
| default | 210 | 30 | Camera, planes, cards |
| heavy | 150 | 34 | Type, crate, big numbers (critically damped: no overshoot) |
| chromeSnap | 260 | 20 | Tiny UI pieces landing (toggle, badge) — the only visible overshoot |

- **Type enters by mask, never opacity.** A word rises out of a hard clip
  line on the heavy spring. Exits are pushes (the next word shoves it out
  of frame) or hard cuts on the beat.
- **Nothing fades in** as its primary entrance. Opacity is allowed only
  for depth dimming of context layers.
- Values with several targets use `track()`; seeded noise (`mulberry32`)
  for cursor paths; no `Math.random`, timers or CSS transitions.

## Rhythm and sound

120 BPM, 4/4: beat = 0.5 s, bar = 2 s, phrase = 8 s. The film is exactly
32 bars (64 s). Every cut is on a beat, every chapter change on a bar line,
every key motion lands on a beat or 8th. The score and all SFX are
synthesized in code from the same cue list the picture uses
(`lib/timeline.js`), so sync is structural, not eyeballed:

- Kick + stab on each hook word; a riser into the crate reveal; a sub
  impact + noise hit + reverb tail on the crate (4.0 s) and on the CTA.
- Key clicks on typing bursts, a soft click on every cursor press, a
  filtered whoosh on every camera whip, pin "drops" and badge ticks
  pitched up a scale in Superpowers.
- Groove density follows the story: sparse (hook), four-on-floor + hats
  (product), 16th hats + arp (Superpowers), drop to sub + pad (Proof),
  resolve (CTA). Master normalized to −14 LUFS.

## Formats from one timeline

`window.seek(t)` reads a layout `L = {W, H, portrait}` from the stage size.
ROI framing is resolution-independent (the camera fits the ROI into the
free region of whichever frame it has), statement bands move from left
third (16:9) to top third (9:16), grids reflow (4×3 → 3×4). Vertical is a
reflow, never a crop.

## Banned (hard lint)

Centered title on a gradient · everything fading in · corner labels,
frame borders · glows on UI chrome · generic particle bursts · bouncy
overshoot on type · dead beats > 1.5 s · readable text < 52 px (16:9) ·
redrawing real dashboard UI from imagination.
