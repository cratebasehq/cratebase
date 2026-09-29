# Style guide — Cratebase launch film

Named look: **Linear / Vercel / Raycast launch-film grammar** — confident
geometric cuts, real product UI as the hero (never redrawn from memory),
one accent color used sparingly, type that moves with intent instead of
decorating. We take that grammar, never anyone's footage or logos.

This guide is derived entirely from Cratebase's own shipped brand system
(`site/src/styles/tokens.css`, `site/src/assets/illustrations/iso.ts`,
`site/src/components/illustrations/*.astro`) — nothing here is invented.
Where the film needs something the marketing site doesn't have (a dark
cinema ground, motion timing), it's a deliberate extension of that system,
called out below.

## Palette

Pulled verbatim from `tokens.css`. The film's default stage is the **dark**
palette — a real second pass in the brand system, not an inversion — because
a launch film needs a cinema-dark ground to make screenshots and kinetic
type read at a glance; the light palette appears only when it's what the
product itself shows (the dashboard's actual chrome is light-mode).

| Token | Hex | Use in the film |
|---|---|---|
| `--color-bg` (dark) | `#0a1622` | Film ground for every non-UI shot (hook, terminal, transitions, CTA) |
| `--color-panel` (dark) | `#101f30` | Raised surfaces behind/around UI screenshots (the "stage" the dashboard sits on) |
| `--color-ink` (dark) | `#eef1f4` | Kinetic type, on dark ground |
| `--color-text-secondary` (dark) | `#93a4b6` | Supporting captions, timestamps-as-copy, de-emphasized labels |
| `--color-hairline` (dark) | `rgba(238,241,244,.14)` | Dividers, the one-pixel seams on morphing containers |
| `--color-crate` | `#ef7d3e` | **Accent only** — the crate mark, cursor highlight, beat-pulse, the one colored line under a headline. Brand rule from `tokens.css`: *"illustrations + mark ONLY, never UI chrome"* — the film obeys this exactly: no orange buttons, no orange UI, no orange gradients. |
| `--color-fog` (light) | `#eef1f4` | Ground under real dashboard screenshots only, because that's the dashboard's real background — never used as a film-graphics ground |
| `--color-harbor` (light-mode ink) | `#0e2238` | Text inside real UI screenshots only (native to the product) |

No gradients anywhere. `tokens.css` defines none, and the banned-look list
forbids "centered title on a gradient" — the safest way to guarantee that
is to never reach for a gradient at all, including as background texture.

## Type

- **Display / kinetic type**: Bricolage Grotesque Variable (`--font-display`
  in `tokens.css`). One weight axis used expressively: 800–900 for hook
  lines and section titles, 500–600 for supporting lines. Never mix in a
  second display face.
- **UI / data / code**: Red Hat Mono Variable (`--font-mono`). Terminal
  text, `{{variable}}` chips, filter-expression fragments, the benchmark
  table, timestamps-as-copy. Exactly the site's own `code, pre, kbd, .mono`
  rule.
- **Scale** (1080p canvas, 16:9 master — see Formats below for how this
  reflows): hook line 132px/0.98 line-height/-0.01em tracking; section
  titles 72px/1.0; UI captions 28px mono; benchmark numbers 96px mono
  tabular (Red Hat Mono has real tabular figures — use them so digits
  don't jitter width frame to frame).
- Sentence case only. No tracked-out all-caps labels, no eyebrow labels
  above headlines, no mid-dot-joined meta strings, no arrow (`→`)
  appended to CTA text — none of these appear anywhere in the real site
  copy, so none appear in the film.
- Max one accented word per line, and only when the product name or a
  real number is the accent (never a generic emphasis word).

## Shot lengths and pacing

Score: **120 BPM**, 4/4, so one beat = 0.5s and one bar = 2s. 66s total
film = 33 bars = 132 beats. Every cut lands on a beat; every chapter
change (a hard cut, see Transitions) lands on a downbeat (bar line).

- Fastest cuts (Hook, Superpowers): 0.5–1 bar per shot (1–2s) — a new
  thing must be readable inside that window, so these shots carry the
  least text.
- Mid cuts (One binary, Data, Auth, Email): 1.5–2.5 bars (3–5s) — enough
  time for one UI action (typing, a rule evaluating, a provider snapping
  in) to read as an action, not a slide.
- Slow shots (Proof, CTA): 3–4 bars (6–8s) — numbers and the final
  lockup need to sit still enough to be read twice.
- Rule of thumb from the brief: a new visual payoff every 3–5s even
  inside a single beat/chapter — no shot runs longer than 5s without
  something changing (a value ticking, a cursor moving, a panel
  resizing).

## Camera language

Everything is 2D (canvas/DOM), so "camera" is simulated depth, never a
literal lens:

- **Parallax dolly**: a background isometric layer (silhouettes drawn
  from the real `iso.ts` projection — warehouses, pallets, crates at
  10–15% opacity) drifts opposite the foreground at a slow constant
  rate (closed-form linear drift, not eased — camera moves read as
  intentional when they're constant-velocity, not spring-settled).
  Never drifts more than ~40px across a shot; this is ambience, not a
  wandering eye.
- **Push-in on reveal**: when a container is about to morph (see
  Transitions), it scales from 0.94→1.0 on a critically-damped spring
  (`k=210, d=30`, no overshoot) over the first 0.3s of the shot — reads
  as the camera settling on the new subject, not a bounce.
- **No shake, no whip-pan blur, no lens flare, no depth-of-field blur.**
  Those all read as generic "cinematic AI video" tells on a UI-heavy
  film; sharp and still is the Linear/Vercel/Raycast tell instead.
- **Static hold for reading**: benchmark numbers and the final CTA lockup
  get zero camera motion at all — stillness is itself a device, used
  exactly twice (Proof, CTA) so it reads as a choice.

## The one morphing container (brand-literal, not generic)

Per the brief's "one container that morphs rather than hard cuts," the
film's recurring shape is **the crate mark itself** — the same isometric
cube geometry as `site/src/assets/illustrations/iso.ts` (`project()`,
`U=20` unit, top/left/right faces). It is the literal product mark, so
using it as the transition device ties motion grammar to brand identity
instead of an arbitrary shape:

1. **Hook**: type assembles into a crate (cube faces spring in, staggered,
   from the real `faces()`/`braces()` paths — see Springs).
2. **One binary**: the crate's top face flattens and widens into the
   terminal window's title bar; the terminal "opens" upward out of the
   crate (one continuous morph, not a cut).
3. **Data → Auth → Email**: the terminal's body panel resizes (spring on
   width/height/corner-radius, via `track()`) into each successive UI
   frame — a collection table, a rule editor, a sign-in card, an email
   editor — without ever leaving frame.
4. **Superpowers**: the panel splits into two morph targets at once
   (search results list + a map card) — this is the one shot allowed to
   have two simultaneous morph targets, because the beat is literally
   about several features arriving together.
5. **Proof / CTA**: the panel flattens back down into a single crate
   silhouette, now static, which is the last frame — a visual rhyme with
   the opening crate (loop-friendly, and doubles as the poster frame).

Hard cuts exist only *between* chapters that the crate isn't actively
mid-morph across (e.g., Auth→Email if the morph already resolved a beat
early) — and even then the cut is masked by a diagonal wipe at the same
angle as the isometric projection's top-face edge (the real geometry's
own angle, not an arbitrary 45°), never a plain crossfade.

## Transition grammar

- **Morph** (primary, ~70% of transitions): a value with one or more
  targets on `track()` — width, height, radius, x, y. Default spring
  `k=210, d=30` (critically damped, no overshoot) for containers; `k=260,
  d=20` (measured ~8% overshoot in `lib/motion.test.mjs`) for small UI chrome snapping into place
  (a tab indicator arriving, a toggle switching, a button in a scrolling list settling).
- **Isometric wipe** (chapter changes only): a hard-edged clip-path wipe
  at the crate top-face angle (`atan2(H, W)` from `iso.ts`'s own
  constants), duration 0.18s, no easing curve — a wipe should feel like
  a cut with direction, not a fade.
- **Text enter**: clip-path reveal in the direction of reading (left
  edge sweeps right), starting the instant the container morph that
  hosts it begins, offset +0.08s so the box is recognizably underway
  before letters appear (matches the course's `swapAlpha` pattern, but
  implemented as a hard clip-path, never opacity).
- **Text exit**: reverse clip-path sweep, completing 0.1s before the next
  morph target lands — never a fade-to-transparent.
- **No cross-dissolves, no fades of any kind, anywhere in the film.**

## Motion quality (springs)

Closed-form damped spring, per the course's `spring(t, k, d)`. Four
presets, matching the course's own vocabulary:

| Preset | k | d | Used for |
|---|---|---|---|
| Snappy | 320 | 28 | Cursor moves, tab indicators, toggle switches |
| Default | 210 | 30 | Containers, cards, the camera push-in |
| Heavy | 150 | 34 | Big type entrances, the crate assembly, the final lockup |
| Chrome-snap | 260 | 20 | Small UI elements landing — a toggle, a tab indicator (tiny visible overshoot — the *only* place overshoot is allowed on non-type, non-crate elements; no logo grid exists in the real product, see shotlist.md Ch. 4) |

Rules carried over directly from the brief and the course:
- **Type never overshoots.** Display type uses Heavy (critically damped
  at that `k`/`d` pair — verify numerically before locking, see
  `lib/motion.js` tests) or a `d` raised further until overshoot is
  provably <0.5px at the type's rendered size.
- Any value with more than one target over the film's timeline (a
  cursor visiting three fields, a panel resizing three times) is a sum
  of one spring per change via `track()` — never a restarted animation.
- Seeded noise (`mulberry32`) only, for the handful of organic touches
  (background silhouette drift jitter, the search-results shimmer) —
  never `Math.random()`, so the render is identical every run.

## Texture

Flat fills only. No grain, no noise texture, no drop shadows beyond a
single hard 1px hairline (`--color-hairline`) offset — matching the real
dashboard's own UI, which uses hairline borders, not soft shadows. The
isometric illustrations' existing shading (top face lit, left/right faces
in progressively deeper harbor shadow, per `CrateBox.astro`) is the only
"depth" texture in the film — it's real brand shading, not an added
effect.

## Banned (from the brief; treated as a hard lint, not a guideline)

Centered title on a gradient · everything fading in · corner labels or
frame borders (timecodes, "01/08" counters, safe-area brackets) · glows
on UI chrome · generic particle bursts · bouncy overshoot on type ·
tracked-out all-caps labels · arrow-suffixed CTAs · mid-dot-joined meta
strings.

## Formats from one timeline

The scene graph is authored against a logical stage size and a layout
function per scene, not fixed 1920×1080 pixels, so 16:9, 9:16 and 1:1 all
call the same `seek(t)` with a different `frame(w,h)` — reflow, not crop.
9:16 stacks what 16:9 places side-by-side (e.g., UI screenshot above its
caption instead of beside it); kinetic type re-wraps to the narrower
measure at the same tracking. See `docs/shotlist.md` per-shot notes for
which shots need an explicit vertical layout (mostly Data, Auth, Email —
the three shots with UI + caption side by side in 16:9).
