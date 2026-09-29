// Virtual 3D camera for flat planes (screenshots, cards, maps).
//
// We build each plane's full CSS `matrix3d` ourselves — perspective
// included — instead of stacking CSS `perspective` + `rotate` + `scale`.
// That lets us project any point on a plane (a rule expression's box from
// capture/shots/*.boxes.json, a field row, a pin) to exact screen
// coordinates, so callouts are real DOM text laid out *outside* the
// transformed plane (always crisp) yet pinned to the detail they label.
//
// Matrices are column-major Float64Array(16), exactly CSS matrix3d order.

import { track } from "./motion.js";

export function ident() {
  const m = new Float64Array(16);
  m[0] = m[5] = m[10] = m[15] = 1;
  return m;
}

export function mul(a, b) {
  const o = new Float64Array(16);
  for (let c = 0; c < 4; c++)
    for (let r = 0; r < 4; r++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
      o[c * 4 + r] = s;
    }
  return o;
}

export function translate(x, y, z = 0) {
  const m = ident();
  m[12] = x;
  m[13] = y;
  m[14] = z;
  return m;
}

export function scale(sx, sy = sx, sz = 1) {
  const m = ident();
  m[0] = sx;
  m[5] = sy;
  m[10] = sz;
  return m;
}

export function rotX(deg) {
  const a = (deg * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
  const m = ident();
  m[5] = c;
  m[6] = s;
  m[9] = -s;
  m[10] = c;
  return m;
}

export function rotY(deg) {
  const a = (deg * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
  const m = ident();
  m[0] = c;
  m[2] = -s;
  m[8] = s;
  m[10] = c;
  return m;
}

export function rotZ(deg) {
  const a = (deg * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
  const m = ident();
  m[0] = c;
  m[1] = s;
  m[4] = -s;
  m[5] = c;
  return m;
}

export function perspective(d) {
  const m = ident();
  m[11] = -1 / d;
  return m;
}

export function chain(...ms) {
  return ms.reduce((a, b) => mul(a, b));
}

export function css(m) {
  let s = "matrix3d(";
  for (let i = 0; i < 16; i++) s += (i ? "," : "") + (Math.abs(m[i]) < 1e-12 ? 0 : +m[i].toPrecision(10));
  return s + ")";
}

/** Project plane-local (x, y, z) through `m` to screen px. */
export function project(m, x, y, z = 0) {
  const X = m[0] * x + m[4] * y + m[8] * z + m[12];
  const Y = m[1] * x + m[5] * y + m[9] * z + m[13];
  const W = m[3] * x + m[7] * y + m[11] * z + m[15];
  return [X / W, Y / W];
}

/**
 * A camera shot: the ROI centre (cx, cy in plane px) is placed at screen
 * point (px, py) with uniform zoom `z`, tilted by rx/ry/rz degrees about
 * that point, seen through `persp` px of perspective.
 */
export function shotMatrix({ cx, cy, z, rx = 0, ry = 0, rz = 0, px, py, persp = 1800, lift = 0 }) {
  return chain(translate(px, py), perspective(persp), translate(0, 0, lift), rotX(rx), rotY(ry), rotZ(rz), scale(z), translate(-cx, -cy));
}

/** Zoom that fits `roi` ({x,y,w,h} in plane px) into `region` ({x,y,w,h} screen px). */
export function fitZoom(roi, region, fill = 0.92) {
  return Math.min(region.w / roi.w, region.h / roi.h) * fill;
}

/**
 * Frame an ROI into a screen region → shot params (without tilt).
 * `minZoom` lets a shot guarantee legibility (e.g. 14px CSS text × dpr 4
 * needs z ≥ 52/56 ≈ 0.93 to render at the 52px floor).
 */
export function frame(roi, region, { fill = 0.92, minZoom = 0, maxZoom = 99 } = {}) {
  const z = Math.min(maxZoom, Math.max(minZoom, fitZoom(roi, region, fill)));
  return { cx: roi.x + roi.w / 2, cy: roi.y + roi.h / 2, z, px: region.x + region.w / 2, py: region.y + region.h / 2 };
}

/**
 * Camera over time: keys = [[t, shotParams], ...]. Each numeric param is
 * tracked with its own spring sum (motion.js track()); zoom is tracked in
 * log space so push-ins read as constant-rate, not accelerating.
 */
export function camTrack(t, keys, k = 210, d = 30) {
  const out = {};
  const names = new Set();
  for (const [, p] of keys) for (const n of Object.keys(p)) names.add(n);
  for (const n of names) {
    let last = keys.find(([, p]) => p[n] !== undefined)[1][n];
    const series = keys.map(([tt, p]) => {
      if (p[n] !== undefined) last = p[n];
      return [tt, n === "z" ? Math.log(last) : last];
    });
    const v = track(t, series, keys.k ?? k, keys.d ?? d);
    out[n] = n === "z" ? Math.exp(v) : v;
  }
  return out;
}
