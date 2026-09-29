# Cratebase launch film

A 64-second launch film rendered entirely from code: one page with a pure
`window.seek(t)` render contract, a virtual 3D camera over real DPR-4
dashboard captures, closed-form springs, a 120 BPM beat grid, and a score +
SFX synthesized in Node from the same cue sheet the picture uses. The
16:9 master and the 9:16 cut are the same timeline laid out by `layout(W, H)`;
vertical is a reflow, not a crop.

Direction: `docs/style_guide.md` · beats: `docs/shotlist.md` · review
rounds and scores: `docs/critique.md`.

## Deliverables (in `out/delivery/`, gitignored)

| File | What |
|---|---|
| `cratebase-launch-16x9.mp4` | 1920×1080, 60 fps, H.264 High yuv420p, AAC, < 20 MB |
| `cratebase-launch-16x9.webm` | same, VP9 + Opus |
| `cratebase-launch-9x16.mp4` | 1080×1920 vertical reflow, 60 fps, H.264 |
| `cratebase-teaser-15s-16x9.mp4` | 15 s cut on beat boundaries from the master |
| `poster-16x9.png`, `poster-9x16.png` | clean last frame |
| `still-1..6-*.png` | key stills |
| `contact-sheet-16x9.png` | final contact sheet (2 fps, 6 across) |

## Pipeline — exact commands

Everything that needs Chromium runs in Docker (the host Chromium lacks
system libs). `render.sh` mounts this folder at `/work` and runs as your
user, so code edits need no image rebuild. ffmpeg and Node 22 are also
used directly on the host (encode, audio, critique sheets).

```bash
cd marketing/video
./fetch-fonts.sh                 # vendor brand + product fonts into assets/fonts/ (once)
./render.sh build                # Docker image: Playwright 1.48 + ffmpeg (once)
```

### 1. Real UI captures (only when the product UI changes)

Never compile the Rust workspace for this — use the released binary.

```bash
curl -fsSL https://cratebase.dev/install.sh | sh
mkdir -p /tmp/cbfilm/run && cd /tmp/cbfilm/run     # empty cwd: dev resolves pb_hooks/pb_migrations from cwd
cratebase dev --http 127.0.0.1:8097 --dir /tmp/cbfilm/data   # note the printed superuser password
# in marketing/video:
CRATEBASE_URL=http://127.0.0.1:8097 CRATEBASE_SUPERUSER_EMAIL=admin@localhost \
  CRATEBASE_SUPERUSER_PASSWORD=<printed> sh capture/seed.sh    # places/posts schema, records, one real welcome mail
docker run --rm --network host --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$(pwd):/work" -w /work \
  -e CRATEBASE_URL=http://127.0.0.1:8097 -e CRATEBASE_SUPERUSER_EMAIL=admin@localhost \
  -e CRATEBASE_SUPERUSER_PASSWORD=<printed> cb-video-engine node capture/capture.mjs
node capture/boxes.mjs hd-places-schema "owner|Geo point"     # measured element rects, for ROIs/callouts
```

Captures are `capture/shots/hd-*.png` (1280 px CSS wide at DPR 4) plus
`hd-*.boxes.json` with the rect of every input, button and text run.

### 2. Stills (look before you animate)

```bash
./render.sh stills --dir out/stills/a 4.05 17.3 27.0 42.9          # 16:9 PNGs
./render.sh stills --w 1080 --h 1920 --dir out/stills/v 17.3 42.9  # 9:16 PNGs
./render.sh stills --every 0.5 --dir out/stills/all                # one per beat
```

Live preview in a desktop browser: serve this folder (`npx serve .`) and open
`index.html?play&t=36` (or `?w=1080&h=1920&play`).

### 3. Score and SFX

```bash
node audio/score.mjs --out out/score.wav --stems   # ~2 s; imports every scene's cues via lib/cues.mjs
sh audio/master.sh                                 # -> out/score-master.wav, -14 LUFS, <= -1 dBTP (linear)
node audio/sync-check.mjs out/stem-sfx.wav         # onset-vs-cue sync report
```

### 4. Masters and deliverables

```bash
./render.sh node render.mjs --w 1920 --h 1080 --fps 60 --workers 4 --out out/master-16x9.mp4
./render.sh node render.mjs --w 1080 --h 1920 --fps 60 --workers 4 --out out/master-9x16.mp4
./encode.sh      # delivery MP4s/WebM, teaser, posters, stills, final contact sheet + ffprobe report
```

`render.mjs` splits the frame range across `--workers` browser contexts,
encodes near-lossless segments (CRF 10) and concatenates them. `--sub N`
averages N subframes per output frame (motion blur); the masters use crisp
60 fps because 2 subframes ghosted fast moves into two visible copies
(critique round 3) — use `--sub 4`+ if you want blur, never 2.
`encode.sh` does the size-tuned delivery encodes from those masters.

### 5. Critique loop (after stills, animatic and full pass — until every score ≥ 8)

```bash
./render.sh node render.mjs --fps 12 --out out/animatic.mp4      # quick full-timeline pass
./encode.sh mux out/animatic.mp4 out/animatic-av.mp4             # add the score
./critique.sh out/animatic-av.mp4 36.2 r1   # out/critique/r1-{contact,strip,phone}.png
```

Open all three sheets, score hook / phone readability / motion / variety /
composition / brand / sound sync, log the 3 worst problems with timestamps
in `docs/critique.md`, fix, repeat.

### Determinism

```bash
./render.sh stills --dir out/stills/detA 13.1 36.9 44.4 60.9
./render.sh stills --dir out/stills/detB 60.9 44.4 36.9 13.1     # reverse order
(cd out/stills/detA && md5sum *.png) > /tmp/a.md5 && (cd out/stills/detB && md5sum -c /tmp/a.md5)
```

No `Math.random`, timers, CSS transitions or carried animation state. Known
residue: Chromium's compositor may rasterize a transformed text layer at a
cached scale, so a few frames differ by ≤ 2/255 on ~0.02 % of channels
(anti-aliasing only) depending on render order.

## Layout

```
index.html            stage + vendored @font-face; main.js boots scenes, window.seek(t)
main.js               scene registry, impact dips, ?w=&h= layout, ?play preview
scenes/*.js           one module per chapter: build(root, L) once, draw(t, state, L) per frame,
                      plus top-level addCue() calls on the beat grid
lib/timeline.js       BPM, bars, chapters, arrangement sections, chords, cue sheet
lib/cues.mjs          Node aggregator: imports scenes so the score sees every cue
lib/motion.js         closed-form spring(), track(), indicator(), mulberry32, iso projection
lib/cam.js            matrix3d camera: shotMatrix, frame(roi, region), camTrack, project()
lib/ui.js             UIPlane: a capture + its boxes.json, spotlight, reveal masks
lib/kit.js            layout(W,H), mask-rise type, statements, cursor, callouts, code cards, crate SVG
lib/serve.mjs         static server + page opener shared by render/stills
audio/                score.mjs (synth + mix), dsp.mjs, theory.mjs, wav.mjs, master.sh, sync-check.mjs
capture/              capture.mjs (DPR 4 + boxes), seed.sh, boxes.mjs, capture-team-board.mjs (v1)
render.mjs stills.mjs render.sh encode.sh critique.sh fetch-fonts.sh Dockerfile
docs/                 style_guide.md, shotlist.md, critique.md
```
