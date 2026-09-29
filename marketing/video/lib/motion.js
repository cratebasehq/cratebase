// Pure motion primitives for the render contract: every value the film
// uses is a closed-form function of time, never a simulation stepped
// frame by frame and never carried state — see docs/style_guide.md
// ("Motion quality") and the render contract in README.md.
//
// This file is loaded by index.html in the browser (as a classic
// <script>, no bundler — route A from motion-course.md) and is plain
// enough to also `import` from a Node test file, which is how
// lib/motion.test.mjs checks the "type never overshoots" rule numerically
// instead of by eye.

/** Closed-form damped harmonic oscillator, 0 -> 1 as t -> +inf. Pure
 * function of time: calling this for frame 812 costs the same as frame 0.
 * k = stiffness, d = damping. z (the damping ratio) < 1 is underdamped
 * (visible overshoot before settling); z >= 1 is treated as critically
 * damped (no overshoot, `d` past the critical value just slows it down). */
export function spring(t, k = 210, d = 30) {
  if (t <= 0) return 0;
  const w0 = Math.sqrt(k);
  const z = d / (2 * w0);
  if (z < 1) {
    const wd = w0 * Math.sqrt(1 - z * z);
    return 1 - Math.exp(-z * w0 * t) * (Math.cos(wd * t) + ((z * w0) / wd) * Math.sin(wd * t));
  }
  return 1 - Math.exp(-w0 * t) * (1 + w0 * t);
}

/** The four named presets from docs/style_guide.md's "Motion quality"
 * table, so scene code reads as intent ("Springs.heavy") instead of bare
 * numbers repeated at every call site. */
export const Springs = {
  snappy: { k: 320, d: 28 },
  default: { k: 210, d: 30 },
  heavy: { k: 150, d: 34 },
  chromeSnap: { k: 260, d: 20 },
};

export function springWith(t, preset) {
  return spring(t, preset.k, preset.d);
}

export const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));

/** A value with more than one target over the timeline is the sum of one
 * spring per change (docs/style_guide.md: "never a restarted animation").
 * `keys`: [[time, value], ...] sorted by time ascending. Returns the
 * value at `t`. Same shape as motion-course.md's `track()`. */
export function track(t, keys, k = 210, d = 30) {
  let v = keys[0][1];
  for (let i = 1; i < keys.length; i++) {
    v += (keys[i][1] - keys[i - 1][1]) * spring(t - keys[i][0], k, d);
  }
  return v;
}

/** A tab/indicator-style shape whose leading and trailing edges settle at
 * different rates, so it visibly stretches instead of just translating.
 * `stops`: [[time, x], ...]. */
export function indicator(t, stops, leadPreset = Springs.snappy, trailPreset = Springs.default) {
  const lead = track(t, stops, leadPreset.k, leadPreset.d);
  const trail = track(t, stops, trailPreset.k, trailPreset.d);
  return { left: Math.min(lead, trail), right: Math.max(lead, trail) };
}

/** Hard clip-path reveal window for text inside a morphing container:
 * enters just after the container morph starts, exits just before the
 * next one lands. Returns 0..1, meant to drive a clip-path inset/percentage,
 * never an opacity (docs/style_guide.md bans fades outright). */
export function wipeWindow(t, tIn, tOut, inDur = 0.12, outDur = 0.1) {
  const enter = clamp((t - tIn - 0.08) / inDur);
  const exit = clamp((tOut - outDur - t) / outDur);
  return Math.min(enter, exit);
}

/** Seamless loop helper: pin the last frame to the first. */
export const loopT = (t, dur) => ((t % dur) + dur) % dur;

/** Deterministic seeded PRNG (mulberry32) — never Math.random(), so two
 * renders of the same frame are byte-identical (see README.md's
 * determinism check). `seed` is any 32-bit integer. */
export function mulberry32(seed) {
  let a = seed | 0;
  return function () {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** The crate mark's isometric projection, lifted verbatim from
 * site/src/assets/illustrations/iso.ts so the film's morphing container
 * (docs/style_guide.md's "one morphing container") is the exact same
 * geometry as the brand mark, not a redrawn approximation. Screen pixels
 * per world unit `U` is a parameter here (the site hardcodes U=20 for a
 * 32x32 mark; the film uses a larger U for a full-frame crate). */
export function isoProject([x, y, z], U = 20) {
  const W = U;
  const H = (U * 5.8) / 11;
  const V = (U * 10.4) / 11;
  return [(x - y) * W, (x + y) * H - z * V];
}

/** The isometric top-face edge angle, in radians — used for the one hard
 * cut in the film (the Superpowers -> Proof wipe), so the wipe's diagonal
 * matches the crate's own geometry instead of an arbitrary 45 degrees. */
export const ISO_EDGE_ANGLE = Math.atan2((20 * 5.8) / 11, 20);
