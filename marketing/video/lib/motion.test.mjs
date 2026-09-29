// Numeric checks for the two hard rules in docs/style_guide.md's "Motion
// quality" section: type must never overshoot, and every render must be
// deterministic. Run with `node lib/motion.test.mjs` (no framework —
// matches the course's own "verify numerically before locking" note).

import assert from "node:assert/strict";
import { spring, Springs, track, mulberry32 } from "./motion.js";

function maxOvershoot(k, d, sampleSeconds = 3, steps = 3000) {
  let max = 0;
  for (let i = 0; i <= steps; i++) {
    const t = (i / steps) * sampleSeconds;
    max = Math.max(max, spring(t, k, d) - 1);
  }
  return max;
}

// Heavy (type + crate assembly + final lockup) must be provably
// non-overshooting: z = d / (2*sqrt(k)) >= 1.
{
  const { k, d } = Springs.heavy;
  const z = d / (2 * Math.sqrt(k));
  assert.ok(z >= 1, `Springs.heavy must be critically/over damped, got z=${z}`);
  const overshoot = maxOvershoot(k, d);
  assert.ok(overshoot <= 0, `Springs.heavy overshot by ${overshoot}`);
  console.log(`ok: Springs.heavy z=${z.toFixed(3)}, overshoot=${overshoot}`);
}

// Default (containers, the camera push-in) is also specified as
// critically damped, no overshoot.
{
  const { k, d } = Springs.default;
  const overshoot = maxOvershoot(k, d);
  assert.ok(overshoot <= 1e-9, `Springs.default overshot by ${overshoot}`);
  console.log(`ok: Springs.default overshoot=${overshoot}`);
}

// Chrome-snap is the one place a small overshoot is allowed by the style
// guide ("tiny overshoot") — assert it exists and stays small (<8% of the
// unit step), not that it's absent.
{
  const { k, d } = Springs.chromeSnap;
  const overshoot = maxOvershoot(k, d);
  assert.ok(overshoot > 0, "Springs.chromeSnap should have a visible overshoot");
  assert.ok(overshoot < 0.1, `Springs.chromeSnap overshoot too large: ${overshoot}`);
  console.log(`ok: Springs.chromeSnap overshoot=${overshoot.toFixed(4)} (expected small, nonzero)`);
}

// track() must reach every key's target value in the limit (each added
// spring's own contribution sums to that key's delta).
{
  const keys = [
    [0, 0],
    [1, 100],
    [2, 40],
  ];
  const late = track(5, keys, Springs.default.k, Springs.default.d);
  assert.ok(Math.abs(late - 40) < 0.01, `track() should settle near 40, got ${late}`);
  console.log(`ok: track() settles at ${late.toFixed(3)}`);
}

// Determinism: mulberry32 with the same seed must reproduce the same
// sequence — this is the whole reason the film never calls Math.random().
{
  const a = mulberry32(7);
  const b = mulberry32(7);
  const seqA = Array.from({ length: 10 }, () => a());
  const seqB = Array.from({ length: 10 }, () => b());
  assert.deepEqual(seqA, seqB, "mulberry32(7) must be reproducible");
  console.log("ok: mulberry32 is deterministic for a fixed seed");
}

console.log("\nAll motion.js checks passed.");
