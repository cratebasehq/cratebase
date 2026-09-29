---
name: motion-reel
description: Make a Cratebase launch/feature/release video rendered from code — real dashboard UI, no licensed music, closed-form springs. Use when the user asks for a launch video, release reel, product demo video, or animated explainer for this repo.
---

# Motion reel (Cratebase)

This is `marketing/video/` turned into a repeatable pipeline, built while
producing the first launch film (see `marketing/video/docs/` for that
film's own style guide, shot list and critique log — read them for a
worked example before starting a new one). The method is
`motion-course.md`'s: a pure `window.seek(t)` render contract, closed-form
springs, a beat grid, synthesized sound, and a critique loop that looks
at its own frames instead of trusting the first pass.

## Inputs to collect first

- What's this video for (a release, a specific feature, a full launch)
  and how long (a launch film is ~60-70s; a single-feature clip can be
  15-20s).
- Format(s): 16:9 always; 9:16/1:1 only if asked.
- Which real screens it needs — cross-check against `README.md`,
  `CHANGELOG.md`, `site/src/content/docs/`, `benchmarks/README.md` for
  what's actually shipped vs. still on a feature branch (see "Ground
  every claim" below).
- Anything from the previous film's `docs/style_guide.md` that should
  change (new brand token, new named look) — default to reusing it
  unchanged, since brand consistency across releases matters more than
  novelty per video.

## Hard rules (carried over from this repo's own film)

- **Real UI only.** Never invent a screen. If a screen doesn't exist as
  shown, either it isn't in the video, or it's a clearly-labeled
  recreation built from the real API/SDK shape (see "Ground every claim"
  and `docs/shotlist.md`'s "RECREATE" convention from the first film).
- **Ground every on-screen claim** in `README.md`, `CHANGELOG.md`,
  `site/src/content/docs/`, or `benchmarks/README.md` — never a
  remembered or estimated number.
- **No licensed music.** Synthesize score + SFX in code, locked to a beat
  grid (120 BPM is this repo's default; see `sfx/` once it exists).
- **Never compile the Rust workspace** from this skill. Use the released
  binary (`curl -fsSL https://cratebase.dev/install.sh | sh`) for the
  demo server, and pass `CRATEBASE_BIN` explicitly to any example app's
  dev script (several fall back to `cargo build` when it's unset — see
  `marketing/video/README.md`).
- **Everything renders in Docker** (`marketing/video/Dockerfile`) — this
  environment's own Chromium is missing system libs and there's no sudo.
- Render contract: pure function of time, no CSS transitions/timers, no
  `Math.random` (seeded noise only), springs from
  `marketing/video/lib/motion.js`, never a restarted animation for a
  value with more than one target (`track()`).
- Banned looks (do not reproduce): centered title on a gradient,
  everything fading in, corner labels/frame borders, glows on UI chrome,
  generic particle bursts, bouncy overshoot on type.

## Pipeline

1. **Assets.** Install the real released binary, run `cratebase dev`
   against a demo schema that exercises whatever this video is about
   (reuse `marketing/video/capture/seed.sh`'s `places`/`posts` schema if
   generic CRUD/auth/search is enough; write a new one if the video is
   about a specific feature that schema doesn't touch). Capture with
   `marketing/video/capture/capture.mjs` (extend its `SHOTS` list, don't
   fork the file, unless the target is a different app entirely like
   `examples/team-board` — then add a sibling script the way
   `capture-team-board.mjs` did).
2. **Plan.** Reuse `marketing/video/docs/style_guide.md` as-is unless the
   brief calls for a new look. Write a **new** `docs/shotlist.md` (or a
   dated variant, e.g. `docs/shotlist-v0.5.0.md`) for this video's own
   beats — don't overwrite the previous film's shot list, it's the
   worked example and the record of what shipped.
3. **Stills.** One key frame per beat via `node stills.mjs` (edit its
   `BEATS` array first). Look at every one before writing more scene
   code.
4. **Animatic.** `node render.mjs --fps 8 --out out/animatic.mp4` (or
   `./render.sh animatic`). Fix pacing before polish.
5. **Full pass + polish + audio.** Wire up SFX/score once picture pacing
   is right, not before — audio synced to placeholder timing gets
   re-synced for free when it's built against the beat grid, not against
   specific frame numbers.
6. **Critique loop, minimum 3 rounds** (see
   `marketing/video/docs/critique.md` for the format this repo uses):
   contact sheet (`fps=2,scale=270:-1,tile=6xN`), a 12-frame strip around
   the fastest action, a 360px-wide phone test. Score 1-10 on hook /
   phone-size readability / motion quality / variety / composition /
   brand accuracy / sound sync. List the 3 worst problems with
   timestamps. Fix. Repeat until every score is 8+.
7. **Render every format from the one timeline**, `./render.sh final`
   equivalent per format — reflow (a scene's own `frame(w,h)` layout),
   never crop a 16:9 render to 9:16.
8. **Deliver**: MP4 (H.264 yuv420p) + WebM (VP9), poster PNG, final
   contact sheet, key stills, and say in your final report what's left
   undone if this is a multi-session job — don't imply "done" if a gate
   was skipped.

## Files this skill expects to find (and update as the pipeline evolves)

```
marketing/video/
  index.html, main.js         # edit main.js's SCENES/timeline for a new video;
                               # index.html's DOM/CSS structure is reusable as-is
  lib/motion.js                # springs/track/mulberry32/isoProject — stable, don't fork
  render.mjs, stills.mjs       # generic — take --dur/--fps/--out, don't fork per video
  capture/                     # extend capture.mjs's SHOTS list; add a sibling script
                                # only for a genuinely different app (see capture-team-board.mjs)
  docs/                        # style_guide.md is reusable; shotlist.md and critique.md
                                # are per-video (name the new ones, don't overwrite)
  Dockerfile, render.sh        # reusable as-is
```
