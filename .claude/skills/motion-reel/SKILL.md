---
name: motion-reel
description: Make a Cratebase launch, release or feature film rendered from code — real dashboard captures under a virtual 3D camera, mega kinetic type, synthesized score locked to a beat grid, 16:9 + 9:16 from one timeline. Use when the user asks for a launch video, release reel, feature clip, product demo video or animated explainer for this repo.
---

# Motion reel (Cratebase)

The engine lives in `marketing/video/` (read its README for exact commands,
`docs/style_guide.md` for the look, `docs/shotlist.md` and
`docs/critique.md` for a worked film). The method is `motion-course.md`'s:
pure `window.seek(t)`, closed-form springs, a beat grid, synthesized sound,
and a critique loop that looks at its own frames.

**One-sentence use:** "Make a 20 s film for <feature> from the v0.X
release notes" → collect inputs below, add a shot list, write one or two
`scenes/*.js`, run the gates.

## Inputs (ask only for what you can't find)

Purpose + length (launch ≈ 64 s = 32 bars; feature clip 16–24 s), formats
(16:9 always, 9:16 almost always), which shipped features — verify each in
`CHANGELOG.md` / `README.md` / `benchmarks/README.md`, and check unreleased
feature branches so those get a "Coming in vX" label, never a silent claim.

## What makes it look expensive (learned the hard way on v1 → v2)

- **Scale is the whole game.** v1 failed at 30 % frame fill. Every shot is
  full-bleed; UI planes are bigger than the frame; hook words are 400 px+
  condensed Bricolage (`font-stretch: 75%`).
- **The camera does the telling.** Frame a region of interest measured from
  `capture/shots/*.boxes.json` (`ui.box("owner = @request.auth.id")`,
  `frame(roi, L.below)`), push in until the detail's glyphs are ≥ 52 px at
  1080p (DPR-4 captures make that crisp), rack between ROIs with `camTrack`
  (log-zoom), tilt 4–16°, dim the rest with `plane.spot()`.
- **Callouts live outside the 3D plane** (`project(m, x, y)` → crisp DOM),
  so they never blur and stay pinned to the detail.
- **Statements go in a top band over a dimmed plane** for UI chapters (a
  side band caps zoom below the readability floor); side band for type-only
  beats.
- **Density target**: a state change every beat in the busy chapter,
  nothing static > 1.5 s anywhere (end on a build, e.g. chips per beat).
- **Honesty is part of the brand**: label benchmark workloads by what they
  measure (the `search` workload is paged listing, not FTS), cite the run,
  mark unreleased features.

## Pipeline (gates — don't skip)

1. **Plan**: new `docs/shotlist-<name>.md` on the 120 BPM grid (beat 0.5 s,
   bar 2 s); every cut on a beat, chapter changes on bar lines.
2. **Capture**: released binary only (`curl … install.sh | sh`; never
   compile the workspace), `cratebase dev --http 127.0.0.1:<free port>` from
   an empty cwd, `capture/seed.sh`, then `capture/capture.mjs` (add SHOTS
   with `steps` for interaction states, e.g. scrolling an editor to a token).
   Example apps with morphing UI (sign-in cards) are better **rebuilt as DOM
   from their source** (same copy, tokens, fonts) than screenshotted.
3. **Scenes**: `scenes/<chapter>.js` exports `[{ id, from, to, z, build(root, L), draw(t, s, L) }]`
   and calls `addCue(t, type, params)` at top level for every sound. Build
   measures text (fitWidth, offsetWidth) once; draw sets styles through
   `S()` (cached writes). Anything with several targets uses `track()`.
4. **Stills** for every beat in both formats → look → fix.
5. **Animatic** (`--fps 12`) + score → `critique.sh` → round 1.
6. **Full pass** (30 fps) → round 2 → fixes; **vertical** full render gets
   its own round.
7. **Audio**: `node audio/score.mjs --stems && sh audio/master.sh &&
   node audio/sync-check.mjs out/stem-sfx.wav` (typing cues: `n` = number of
   picture reveal steps).
8. **Masters** at 60 fps `--sub 2`, then `./encode.sh` for every deliverable.

## Critique loop (minimum 3 rounds, every score ≥ 8)

`./critique.sh <render> <fastest-action-time> <tag>` → contact (2 fps, 6
across), strip (12 frames), phone (360 px). Actually open them. Score hook,
phone readability, motion, variety, composition, brand, sound sync; log the
3 worst problems with timestamps in `docs/critique.md`; fix; repeat. Sound
sync without listening = onset analysis — say so in the log.

## Gotchas

- `.abs` sets `left:0` — use `left:"auto"` when positioning with `right`.
- Never use `clip-path` on a zero-size absolute container (clips everything).
- Don't measure with `getBoundingClientRect` inside draw for layout that
  must be deterministic (transforms leak in); measure `offset*` at build.
- Different worker splits change H.264 keyframes: check determinism on PNG
  stills, not on encoded segments.
- Docker images can get pruned between sessions: `./render.sh build`.
- Glob `rm` of output folders can be blocked by safety checks: write each
  still batch to its own `--dir` instead of deleting.
- zsh doesn't word-split `$VAR` — pass times literally.
- Banned: centered title on gradient, everything fading in, corner labels,
  glows on UI chrome, particle bursts, overshoot on type, dead beats,
  text < 52 px at 1080p that the viewer must read.
