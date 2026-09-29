// audio/dsp.mjs — small zero-dependency DSP toolkit used by score.mjs.
// Deterministic (mulberry32 PRNG, never Math.random), Float32Array based,
// no per-sample object allocation in hot loops.

export const TWO_PI = Math.PI * 2;

// ---------------------------------------------------------------- utility --

export function clamp(x, lo, hi) {
  return x < lo ? lo : x > hi ? hi : x;
}
export function lerp(a, b, t) {
  return a + (b - a) * t;
}
export function dbToGain(db) {
  return Math.pow(10, db / 20);
}
export function midiToFreq(m) {
  return 440 * Math.pow(2, (m - 69) / 12);
}

// ------------------------------------------------------------- PRNG (seeded) --

// mulberry32 — tiny, fast, deterministic 32-bit PRNG. Never use Math.random
// anywhere in the score so renders are byte-identical run to run.
export function mulberry32(seed) {
  let a = seed >>> 0;
  return function rand() {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** White noise sample in [-1, 1) from a mulberry32-style rand(). */
export function noiseSample(rand) {
  return rand() * 2 - 1;
}

// ----------------------------------------------------------- oscillators --

/** Naive sine — cheap and never needs band-limiting. */
export function sine(phase) {
  return Math.sin(phase);
}

// PolyBLEP band-limited saw/square. `phase01` in [0,1), `dt` = freq/sampleRate.
function polyBlep(t, dt) {
  if (t < dt) {
    t /= dt;
    return t + t - t * t - 1;
  } else if (t > 1 - dt) {
    t = (t - 1) / dt;
    return t * t + t + t + 1;
  }
  return 0;
}
export function saw(phase01, dt) {
  let v = 2 * phase01 - 1;
  v -= polyBlep(phase01, dt);
  return v;
}
export function square(phase01, dt, pw = 0.5) {
  let v = phase01 < pw ? 1 : -1;
  v += polyBlep(phase01, dt);
  let t2 = phase01 + (1 - pw);
  if (t2 >= 1) t2 -= 1;
  v -= polyBlep(t2, dt);
  return v;
}

// -------------------------------------------------------------- envelopes --

/** Percussive exponential decay, value at sample index i (sampleRate sr). */
export function expDecay(i, sr, decaySec, curve = 1) {
  const t = i / sr;
  return Math.exp((-t / Math.max(decaySec, 1e-6)) * curve);
}

/** Linear attack ramp (0..1) over attackSec, else 1. */
export function attackRamp(i, sr, attackSec) {
  if (attackSec <= 0) return 1;
  const t = i / sr;
  return t >= attackSec ? 1 : t / attackSec;
}

/**
 * Full ADSR, value at sample index i for a note of total length `len`
 * samples. sustainLevel held from end-of-decay until release begins at
 * `len - releaseSamples`.
 */
export function adsrAt(i, len, sr, { a = 0.005, d = 0.08, s = 0.7, r = 0.15 } = {}) {
  const aN = a * sr;
  const dN = aN + d * sr;
  const rN = r * sr;
  const relStart = Math.max(dN, len - rN);
  if (i < aN) return aN <= 0 ? 1 : i / aN;
  if (i < dN) return lerp(1, s, (i - aN) / Math.max(dN - aN, 1));
  if (i < relStart) return s;
  const rt = i - relStart;
  const relLen = Math.max(len - relStart, 1);
  if (rt >= relLen) return 0;
  return s * (1 - rt / relLen);
}

// ------------------------------------------------------------- one-poles --

export class OnePoleLP {
  constructor(sr) {
    this.sr = sr;
    this.z = 0;
    this.a = 0;
  }
  setFreq(fc) {
    this.a = Math.exp((-TWO_PI * fc) / this.sr);
  }
  process(x) {
    this.z = (1 - this.a) * x + this.a * this.z;
    return this.z;
  }
}
export class OnePoleHP {
  constructor(sr) {
    this.sr = sr;
    this.zx = 0;
    this.zy = 0;
    this.a = 0;
  }
  setFreq(fc) {
    this.a = Math.exp((-TWO_PI * fc) / this.sr);
  }
  process(x) {
    this.zy = this.a * (this.zy + x - this.zx);
    this.zx = x;
    return this.zy;
  }
}

// --------------------------------------------------------- biquad (RBJ) --

export class Biquad {
  constructor() {
    this.b0 = 1;
    this.b1 = 0;
    this.b2 = 0;
    this.a1 = 0;
    this.a2 = 0;
    this.x1 = 0;
    this.x2 = 0;
    this.y1 = 0;
    this.y2 = 0;
  }
  lowpass(sr, fc, q = 0.707) {
    this._coeffs(sr, fc, q, "lp");
  }
  highpass(sr, fc, q = 0.707) {
    this._coeffs(sr, fc, q, "hp");
  }
  bandpass(sr, fc, q = 1.0) {
    this._coeffs(sr, fc, q, "bp");
  }
  _coeffs(sr, fc, q, type) {
    fc = clamp(fc, 10, sr * 0.49);
    const w0 = (TWO_PI * fc) / sr;
    const cw = Math.cos(w0);
    const sw = Math.sin(w0);
    const alpha = sw / (2 * q);
    let b0, b1, b2, a0, a1, a2;
    if (type === "lp") {
      b0 = (1 - cw) / 2;
      b1 = 1 - cw;
      b2 = (1 - cw) / 2;
      a0 = 1 + alpha;
      a1 = -2 * cw;
      a2 = 1 - alpha;
    } else if (type === "hp") {
      b0 = (1 + cw) / 2;
      b1 = -(1 + cw);
      b2 = (1 + cw) / 2;
      a0 = 1 + alpha;
      a1 = -2 * cw;
      a2 = 1 - alpha;
    } else {
      b0 = sw / 2;
      b1 = 0;
      b2 = -sw / 2;
      a0 = 1 + alpha;
      a1 = -2 * cw;
      a2 = 1 - alpha;
    }
    this.b0 = b0 / a0;
    this.b1 = b1 / a0;
    this.b2 = b2 / a0;
    this.a1 = a1 / a0;
    this.a2 = a2 / a0;
  }
  process(x) {
    const y = this.b0 * x + this.b1 * this.x1 + this.b2 * this.x2 - this.a1 * this.y1 - this.a2 * this.y2;
    this.x2 = this.x1;
    this.x1 = x;
    this.y2 = this.y1;
    this.y1 = y;
    return y;
  }
}

// ------------------------------------------------------------ stereo delay --

export class StereoDelay {
  constructor(sr, timeL, timeR, feedback = 0.3, mix = 0.3) {
    this.bufL = new Float32Array(Math.max(1, Math.ceil(timeL * sr)));
    this.bufR = new Float32Array(Math.max(1, Math.ceil(timeR * sr)));
    this.wL = 0;
    this.wR = 0;
    this.fb = feedback;
    this.mix = mix;
  }
  process(xl, xr) {
    const dl = this.bufL[this.wL];
    const dr = this.bufR[this.wR];
    this.bufL[this.wL] = xl + dl * this.fb;
    this.bufR[this.wR] = xr + dr * this.fb;
    this.wL = (this.wL + 1) % this.bufL.length;
    this.wR = (this.wR + 1) % this.bufR.length;
    return [xl + dl * this.mix, xr + dr * this.mix];
  }
}

// -------------------------------------------------- Freeverb-style reverb --

const COMB_BASE = [1557, 1617, 1491, 1422, 1277, 1356, 1188, 1116];
const AP_BASE = [225, 556, 441, 341];
const STEREO_SPREAD = 23;

class Comb {
  constructor(size, fb, damp) {
    this.buf = new Float32Array(size);
    this.i = 0;
    this.fb = fb;
    this.damp = damp;
    this.store = 0;
  }
  process(x) {
    const y = this.buf[this.i];
    this.store = y * (1 - this.damp) + this.store * this.damp;
    this.buf[this.i] = x + this.store * this.fb;
    this.i++;
    if (this.i >= this.buf.length) this.i = 0;
    return y;
  }
}
class Allpass {
  constructor(size, fb = 0.5) {
    this.buf = new Float32Array(size);
    this.i = 0;
    this.fb = fb;
  }
  process(x) {
    const bufout = this.buf[this.i];
    const y = -x + bufout;
    this.buf[this.i] = x + bufout * this.fb;
    this.i++;
    if (this.i >= this.buf.length) this.i = 0;
    return y;
  }
}

/** Mono-in, stereo-out Schroeder/Freeverb-style reverb, rendered in one pass. */
export class Reverb {
  constructor(sr, { roomSize = 0.84, damp = 0.3 } = {}) {
    const scale = sr / 44100;
    this.combsL = COMB_BASE.map((s, idx) => new Comb(Math.round(s * scale) + idx, roomSize, damp));
    this.combsR = COMB_BASE.map((s, idx) => new Comb(Math.round((s + STEREO_SPREAD) * scale) + idx, roomSize, damp));
    this.apL = AP_BASE.map((s) => new Allpass(Math.round(s * scale)));
    this.apR = AP_BASE.map((s) => new Allpass(Math.round((s + STEREO_SPREAD) * scale)));
  }
  /** Render a whole mono send buffer into a wet stereo pair (same length). */
  renderBuffer(input) {
    const N = input.length;
    const outL = new Float32Array(N);
    const outR = new Float32Array(N);
    const combsL = this.combsL, combsR = this.combsR, apL = this.apL, apR = this.apR;
    for (let i = 0; i < N; i++) {
      const x = input[i];
      if (x === 0) {
        // still have to advance the filters' internal state? No input means
        // silence propagating through is fine to fast-path only when the
        // whole tail has already decayed; cheap heuristic: never skip, the
        // combs need their delay lines pumped every sample regardless.
      }
      let l = 0, r = 0;
      for (let k = 0; k < combsL.length; k++) l += combsL[k].process(x);
      for (let k = 0; k < combsR.length; k++) r += combsR[k].process(x);
      for (let k = 0; k < apL.length; k++) l = apL[k].process(l);
      for (let k = 0; k < apR.length; k++) r = apR[k].process(r);
      outL[i] = l * 0.015;
      outR[i] = r * 0.015;
    }
    return { L: outL, R: outR };
  }
}

// ------------------------------------------------------- soft clip / limiter --

export function softClip(x, thresh = 0.8) {
  const ax = Math.abs(x);
  if (ax <= thresh) return x;
  const over = ax - thresh;
  const y = thresh + over / (1 + over * over * 4);
  return x < 0 ? -y : y;
}

/** Simple feed-forward peak limiter: instant attack, exponential release. */
export class Limiter {
  constructor(sr, { threshold = 0.891, release = 0.1 } = {}) {
    this.threshold = threshold;
    this.g = 1;
    this.relCoef = Math.exp(-1 / (release * sr));
  }
  process(x) {
    const ax = Math.abs(x);
    let targetG = 1;
    if (ax * this.g > this.threshold) targetG = this.threshold / ax;
    if (targetG < this.g) this.g = targetG;
    else this.g = targetG + (this.g - targetG) * this.relCoef;
    return x * this.g;
  }
  /** Link L/R so gain reduction never shifts the stereo image. */
  processStereo(l, r) {
    const ax = Math.max(Math.abs(l), Math.abs(r));
    let targetG = 1;
    if (ax * this.g > this.threshold) targetG = this.threshold / ax;
    if (targetG < this.g) this.g = targetG;
    else this.g = targetG + (this.g - targetG) * this.relCoef;
    return [l * this.g, r * this.g];
  }
}

// ------------------------------------------------------------- duck envelope --

/**
 * Builds a gain envelope (Float32Array of length N, sr samples/sec) that
 * dips to `dbToGain(-depthDb)` at each event time (seconds) with a fast
 * attack and exponential-ish release, combined via min() across events.
 */
export function buildDuckEnvelope(N, sr, times, { depthDb = 6, attack = 0.005, release = 0.18 } = {}) {
  const env = new Float32Array(N).fill(1);
  const floor = dbToGain(-depthDb);
  const aN = Math.max(1, Math.round(attack * sr));
  const rN = Math.max(1, Math.round(release * sr));
  for (const t of times) {
    const t0 = Math.round(t * sr);
    if (t0 >= N) continue;
    const aEnd = Math.min(N, t0 + aN);
    for (let i = Math.max(0, t0); i < aEnd; i++) {
      const v = lerp(1, floor, (i - t0) / aN);
      if (v < env[i]) env[i] = v;
    }
    const rEnd = Math.min(N, aEnd + rN);
    for (let i = aEnd; i < rEnd; i++) {
      const p = (i - aEnd) / rN;
      const v = floor + (1 - floor) * (1 - Math.pow(1 - p, 3));
      if (v < env[i]) env[i] = v;
    }
  }
  return env;
}
