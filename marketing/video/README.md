# Cratebase launch film

"Stop rebuilding your backend." — a 66s product film built entirely from
code (route A from `motion-course.md`: one `index.html`, `window.seek(t)`,
Playwright frame capture, ffmpeg encode). See `docs/style_guide.md` and
`docs/shotlist.md` for the direction, `docs/critique.md` for the review
log. This is a **multi-session production** — see "Status" below for
exactly what's done and what isn't yet.

## Everything runs in Docker

This machine's own Chromium is missing system libs and there's no sudo,
so Playwright (capture and render both) always runs inside the image
built from `Dockerfile`, never on the host. `render.sh` wraps every
command below.

```bash
cd marketing/video
docker build -t cb-video-engine -f Dockerfile .
```

## 1. Stand up the real demo instance

```bash
curl -fsSL https://cratebase.dev/install.sh | sh    # real v0.4.0 binary, not built from source
cratebase dev --dir /tmp/cb-film-demo                # note the printed superuser email/password

CRATEBASE_SUPERUSER_EMAIL=admin@localhost \
CRATEBASE_SUPERUSER_PASSWORD=<printed password> \
marketing/video/capture/seed.sh                       # creates places/posts, seeds demo data,
                                                       # configures all 12 OAuth presets + magic
                                                       # link/OTP/TOTP on the users collection
```

Known gotcha: `cratebase dev`/`serve` resolve `pb_hooks`/`pb_migrations`
relative to the **current working directory**, not `--dir` — always `cd`
into an empty directory first, or a stray `pb_migrations` elsewhere on
the machine will collide.

Also known: background shells in some automation harnesses don't survive
between tool calls even with `nohup`/`disown` — if the server keeps
disappearing, run it in a real background job / a separate terminal.

## 2. Capture the real UI

```bash
docker run --rm --network host \
  -e CRATEBASE_URL=http://127.0.0.1:8090 \
  -e CRATEBASE_SUPERUSER_EMAIL=admin@localhost \
  -e CRATEBASE_SUPERUSER_PASSWORD=<printed password> \
  -v "$(pwd)/marketing/video/capture/shots:/app/capture/shots" \
  cb-video-engine node capture/capture.mjs
```

Produces `capture/shots/*.png` (gitignored — regenerate, don't commit)
at `deviceScaleFactor: 2`. See that script's header for the two real
API-shape gotchas it works around (`fields` not `schema`; a relation's
`collectionId` is flat, not nested under `options`).

Optional: the real tabbed sign-in card from `examples/team-board` (used
in Auth, shot 4.2) needs that example running separately:

```bash
cd examples/team-board && bun install
CRATEBASE_BIN="$HOME/.local/bin/cratebase" CRATEBASE_PORT=8095 \
  CB_DATA_DIR=/tmp/cb-team-board-demo/pb_data bun run scripts/dev.ts
# in another shell, once it prints its Vite URL:
docker run --rm --network host \
  -v "$(pwd)/marketing/video/capture/shots:/app/capture/shots" \
  -v "$(pwd)/marketing/video/capture/capture-team-board.mjs:/app/capture/capture-team-board.mjs" \
  cb-video-engine node capture/capture-team-board.mjs
```

Note `CRATEBASE_BIN` explicitly: without it, `examples/team-board`'s dev
script falls back to building `target/debug/cratebase` with `cargo
build` if that binary is missing — this repo's rule is **never compile
the Rust workspace** here (another agent may be building it), so always
point `CRATEBASE_BIN` at the installed release binary.

## 3. Render

```bash
# Stills — one PNG per beat, fast (no ffmpeg encode)
docker run --rm -v "$(pwd)/marketing/video/out:/app/out" cb-video-engine node stills.mjs

# Animatic — low fps, full timeline, for the critique loop
docker run --rm -v "$(pwd)/marketing/video/out:/app/out" \
  cb-video-engine node render.mjs --fps 8 --out out/animatic.mp4

# Full render (later gate — polish + audio pass not done yet, see Status)
docker run --rm -v "$(pwd)/marketing/video/out:/app/out" \
  cb-video-engine node render.mjs --fps 30 --sub 4 --out out/cratebase-launch-16x9.mp4
```

`--sub 4` averages 4 subframes per output frame for motion blur (the
course's `tmix` pattern) — expensive, so left off (`--sub 1`) for
stills/animatic and only turned on for the final render.

Docker writes `out/` as root (Playwright base image runs as root); clean
up with `docker run --rm -v "$(pwd)/marketing/video/out:/shots" cb-video-engine rm -rf /shots/*`
if the host user can't delete a file directly.

## Critique loop (course step 11)

```bash
ffmpeg -i out/animatic.mp4 -vf "fps=2,scale=270:-1,tile=6x9" -frames:v 1 out/contact.png
ffmpeg -ss <fastest-action-timestamp> -i out/animatic.mp4 -vf "scale=320:-1,tile=12x1" -frames:v 1 out/strip.png
ffmpeg -i out/animatic.mp4 -vf "fps=1,scale=360:-1,tile=5x3" -frames:v 1 out/phone.png
```

Look at all three, score 1-10 on hook / phone-size readability / motion
quality / variety / composition / brand accuracy / sound sync, log in
`docs/critique.md`, fix the 3 worst problems, repeat until every score
is 8+. See `docs/critique.md` for the rounds run so far.

## Determinism check

```bash
docker run --rm -v "$(pwd)/marketing/video/out:/app/out" cb-video-engine node render.mjs --dur 5 --fps 30 --out out/det-a.mp4
docker run --rm -v "$(pwd)/marketing/video/out:/app/out" cb-video-engine node render.mjs --dur 5 --fps 30 --out out/det-b.mp4
md5sum out/det-a.mp4 out/det-b.mp4   # must match — no Math.random, no timers, no carried state
```

## Repo layout

```
marketing/video/
  index.html, main.js        # the render contract: window.seek(t)
  lib/motion.js               # spring(), track(), presets, mulberry32, isoProject
  lib/motion.test.mjs          # numeric checks (node lib/motion.test.mjs)
  render.mjs, stills.mjs      # Playwright -> ffmpeg pipelines
  capture/                    # asset capture against the real dashboard + examples/team-board
  docs/style_guide.md         # palette, type, shot lengths, transition grammar, camera, texture
  docs/shotlist.md            # every shot: frames, camera, text, SFX, with corrections logged
  docs/critique.md            # review rounds, scores, fixes
  Dockerfile, render.sh       # everything above runs in here
  out/                        # rendered output (gitignored)
```

## Status (as of this session)

Done: plan gate (style guide + shot list), asset capture gate (real
v0.4.0 dashboard + `examples/team-board`, 14 screenshots, demo schema
seeded and reproducible), render engine (springs/track/determinism
tested, full 66s timeline wired end to end covering all 8 chapters),
stills gate (`out/stills/01..08-*.png`), one critique round on a low-fps
animatic with a verified fix pass (see `docs/critique.md` — Search went
from a static screenshot to a real re-rank animation; the four
Superpowers recreations went from near-empty to labeled and captioned),
and the reusable `.claude/skills/motion-reel/SKILL.md` pipeline skill.

Not done yet (left for the next session, deliberately — "don't rush to a
final render"):
- A full round 2/3 critique (fresh problem hunt + rescoring on all seven
  axes) — round 1 only reached 8+ on brand accuracy; sound sync can't
  score honestly until there's audio.
- TOTP (25.5-28.0s) is still the weakest single shot — round 1's
  problem #3, not yet fixed.
- Screenshot crop framing — several real captures show more whitespace
  than signal at scale and shrink to illegible at phone size; needs
  per-shot `object-position`/zoom, not a global fix.
- Audio: no score or SFX synthesized yet. The whole film is currently
  silent; every beat is still readable without sound (the brief's
  silent-friendly requirement), but the beat grid isn't sonically locked
  to anything yet.
- Multi-format reflow: only the 16:9 master layout exists. 9:16 and 1:1
  need their own `frame(w,h)` layout per scene, not a crop.
- Deliverables not yet produced: final MP4s in both codecs, the vertical
  cut, the 15s teaser, the poster PNG, the final contact sheet, key
  stills for delivery (as opposed to critique).
