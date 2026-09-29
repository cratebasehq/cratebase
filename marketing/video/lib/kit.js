// Shared building blocks for scenes: DOM helpers, the layout function,
// mask-rise type, the cursor, callouts pinned to projected points, and
// code cards. Everything here is stateless per frame: scenes call these
// from draw(t) with the current time and get the same pixels every run.

import { spring, clamp, track, Springs } from "./motion.js";

// ---------------------------------------------------------------- DOM
export function el(tag, cls, parent, html) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html !== undefined) e.innerHTML = html;
  if (parent) parent.appendChild(e);
  return e;
}

/** Style setter that skips redundant writes (big win when rendering 4k frames). */
export function S(e, props) {
  const c = e.__s || (e.__s = {});
  for (const k in props) {
    const v = props[k];
    if (c[k] !== v) {
      c[k] = v;
      e.style[k] = v;
    }
  }
}
export const px = (v) => `${+v.toFixed(2)}px`;
export function place(e, x, y, extra = "") {
  S(e, { transform: `translate3d(${px(x)},${px(y)},0)${extra}` });
}
export function show(e, on) {
  S(e, { display: on ? "" : "none" });
}
export function text(e, s) {
  if (e.__t !== s) {
    e.__t = s;
    e.textContent = s;
  }
}
export function html(e, s) {
  if (e.__h !== s) {
    e.__h = s;
    e.innerHTML = s;
  }
}

// ---------------------------------------------------------------- layout
/**
 * The layout function every scene reads. `u` is the scale unit (1 at
 * 1080 on the short side). `band` is where statements live, `ui` the
 * region the camera frames UI into — side by side in 16:9, stacked in 9:16.
 */
export function layout(W, H) {
  const portrait = H > W;
  const u = Math.min(W, H) / 1080;
  const m = (portrait ? 64 : 96) * u;
  const band = portrait ? { x: m, y: 120 * u, w: W - 2 * m, h: H * 0.27 } : { x: m, y: 0, w: W * 0.34, h: H };
  const ui = portrait ? { x: 0, y: H * 0.31, w: W, h: H * 0.69 } : { x: W * 0.35, y: 0, w: W * 0.65, h: H };
  // UI chapters: a top statement band and the UI framed full-width below it
  // (the side band would cap dashboard zoom below the 52px glyph floor).
  const top = { x: m, y: (portrait ? 110 : 64) * u, h: portrait ? H * 0.2 : H * 0.2 };
  const below = portrait ? { x: 0, y: H * 0.25, w: W, h: H * 0.75 } : { x: 0, y: H * 0.22, w: W, h: H * 0.78 };
  return { W, H, portrait, u, m, band, ui, top, below, full: { x: 0, y: 0, w: W, h: H } };
}

// ---------------------------------------------------------------- time
export const prog = (t, a, b) => clamp((t - a) / (b - a));
export const heavy = (t) => spring(t, Springs.heavy.k, Springs.heavy.d);
export const snappy = (t) => spring(t, Springs.snappy.k, Springs.snappy.d);
export const soft = (t) => spring(t, Springs.default.k, Springs.default.d);
export const chrome = (t) => spring(t, Springs.chromeSnap.k, Springs.chromeSnap.d);
export const lerp = (a, b, p) => a + (b - a) * p;
export const easeIn = (p) => p * p * p;
export const easeOut = (p) => 1 - Math.pow(1 - p, 3);
export function mixColor(a, b, p) {
  const pa = hex(a), pb = hex(b);
  const c = pa.map((v, i) => Math.round(v + (pb[i] - v) * clamp(p)));
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}
function hex(h) {
  const n = parseInt(h.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

// ---------------------------------------------------------------- type
/**
 * A line of display type that rises out of a hard mask (never fades).
 * Returns { wrap, inner }. `rise(line, p)` with p 0..1 (spring output).
 */
export function maskLine(parent, cls, content) {
  const wrap = el("div", "abs clip nowrap " + (cls || ""), parent);
  const inner = el("div", "", wrap, content);
  S(inner, { willChange: "auto" });
  return { wrap, inner };
}
export function rise(line, p, dir = 1) {
  S(line.inner, { transform: `translate3d(0,${(1 - p) * 125 * dir}%,0)` });
}
/** Fit an element's font-size so its rendered width is `w` (measured, fonts loaded). */
export function fitWidth(e, w, base = 200) {
  S(e, { fontSize: base + "px" });
  const r = e.getBoundingClientRect().width || 1;
  const size = (base * w) / r;
  S(e, { fontSize: size.toFixed(2) + "px" });
  return size;
}

/** Statement: stacked lines rising one after another from a start time. */
export function statement(parent, lines, cls = "stmt") {
  const box = el("div", "abs", parent);
  const ls = lines.map((l) => maskLine(box, cls, l));
  return { box, ls };
}
export function drawStatement(st, t, tIn, tOut, { x, y, size, color, lh = 0.98, stagger = 0.12, gap = 0 }) {
  const on = t >= tIn - 0.01 && t < tOut;
  show(st.box, on);
  if (!on) return;
  place(st.box, x, y);
  st.ls.forEach((l, i) => {
    S(l.wrap, { fontSize: px(size), color, top: px(i * size * lh + i * gap), paddingBottom: px(size * 0.12) });
    const pin = heavy(t - tIn - i * stagger);
    // exit: push up out of the mask, 0.18s before tOut
    const pout = tOut < 900 ? snappy(t - (tOut - 0.2)) : 0;
    S(l.inner, { transform: `translate3d(0,${((1 - pin) * 125 - pout * 125).toFixed(2)}%,0)` });
  });
}

// ---------------------------------------------------------------- cursor
const CURSOR_SVG = `<svg width="44" height="54" viewBox="0 0 22 27"><path d="M2 1.5v20.2l5.1-4.9 3.3 7.7 3.4-1.5-3.3-7.5h7.1z" fill="#fff" stroke="#0a1622" stroke-width="1.5" stroke-linejoin="round"/></svg>`;
export function cursor(parent) {
  const c = el("div", "abs", parent);
  const ring = el("div", "abs", c);
  S(ring, { width: "64px", height: "64px", left: "-32px", top: "-32px", borderRadius: "50%", border: "3px solid #ef7d3e" });
  const arrow = el("div", "abs", c, CURSOR_SVG);
  S(arrow, { left: "-4px", top: "-2px", filter: "drop-shadow(0 6px 10px rgba(0,0,0,.35))" });
  return { c, ring, arrow };
}
/**
 * path: [[t, x, y], ...] screen px (track()ed with the snappy spring);
 * clicks: [t, ...] — the arrow dips and a ring expands from the tip.
 */
export function drawCursor(cur, t, path, clicks = [], scale = 1, visible = true) {
  show(cur.c, visible);
  if (!visible) return;
  const x = track(t, path.map((p) => [p[0], p[1]]), 170, 24);
  const y = track(t, path.map((p) => [p[0], p[2]]), 170, 24);
  let press = 0, ringP = -1;
  for (const c of clicks) {
    const d = t - c;
    if (d > -0.06 && d < 0.14) press = Math.max(press, 1 - Math.abs(d - 0.02) / 0.1);
    if (d >= 0 && d < 0.45) ringP = d / 0.45;
  }
  place(cur.c, x, y, ` scale(${scale})`);
  S(cur.arrow, { transform: `scale(${(1 - 0.12 * clamp(press)).toFixed(3)})` });
  S(cur.ring, { display: ringP >= 0 ? "" : "none", transform: `scale(${(0.3 + ringP * 1.1).toFixed(3)})`, opacity: (1 - ringP).toFixed(3) });
}

// ---------------------------------------------------------------- callouts
/**
 * A callout pinned to a projected point: a crate dot on the detail, a
 * hairline leader, and a label card. Pops in (snappy scale), never fades.
 */
export function callout(parent, label, { mono = false, dark = true } = {}) {
  const g = el("div", "abs", parent);
  const line = el("div", "abs", g);
  const dot = el("div", "abs", g);
  const card = el("div", "abs nowrap", g, label);
  S(g, { display: "none" });
  S(dot, { width: "18px", height: "18px", left: "-9px", top: "-9px", borderRadius: "50%", background: "#ef7d3e", boxShadow: "0 0 0 6px rgba(239,125,62,.28)" });
  S(line, { height: "3px", background: dark ? "#eef1f4" : "#0e2238", transformOrigin: "0 50%" });
  S(card, {
    fontFamily: mono ? "var(--mono)" : "var(--display)",
    fontWeight: mono ? "600" : "700",
    fontStretch: "92%",
    letterSpacing: mono ? "0" : "-0.02em",
    padding: "0.28em 0.55em 0.34em",
    borderRadius: "0.32em",
    background: dark ? "#eef1f4" : "#0e2238",
    color: dark ? "#0a1622" : "#eef1f4",
    boxShadow: "0 18px 40px rgba(0,0,0,.35)",
    transformOrigin: "0 50%",
  });
  return { g, line, dot, card };
}
/** anchor = [x,y] screen; offset = [dx,dy] to the card's left-middle. */
export function drawCallout(co, t, tIn, tOut, anchor, offset, size = 64) {
  const on = t >= tIn && t < tOut;
  show(co.g, on);
  if (!on) return;
  const p = snappy(t - tIn);
  const pout = snappy(t - (tOut - 0.16));
  const s = clamp(p - pout);
  place(co.g, anchor[0], anchor[1]);
  const [dx, dy] = offset;
  const len = Math.hypot(dx, dy);
  const ang = (Math.atan2(dy, dx) * 180) / Math.PI;
  S(co.line, { width: px(len * clamp(p * 1.3 - pout)), transform: `rotate(${ang.toFixed(2)}deg)` });
  S(co.dot, { transform: `scale(${s.toFixed(3)})` });
  S(co.card, { fontSize: px(size), transform: `translate3d(${px(dx)},${px(dy)},0) translateY(-50%) scale(${clamp(p * 1.15 - pout).toFixed(3)})` });
}

// ---------------------------------------------------------------- code
const KW = /\b(const|await|import|from|interface|export|type|string|async|return|new)\b/g;
/** Tiny TS highlighter for code cards (brand-restrained palette). */
export function hl(src) {
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  return src
    .split("\n")
    .map((line) => {
      let out = "";
      const re = /("[^"]*"|'[^']*'|`[^`]*`)|(\/\/.*$)|(\b[a-zA-Z_$][\w$]*)(?=\s*\()|(\b[a-zA-Z_$][\w$]*)(?=\s*:)|(\b(?:const|await|import|from|interface|export|type|async|return|new)\b)|([{}()[\],.;:=])/g;
      let last = 0, m;
      while ((m = re.exec(line))) {
        out += esc(line.slice(last, m.index));
        const s = esc(m[0]);
        if (m[1]) out += `<span style="color:#f6b183">${s}</span>`;
        else if (m[2]) out += `<span style="color:#5d7086">${s}</span>`;
        else if (m[3]) out += `<span style="color:#ffffff;font-weight:600">${s}</span>`;
        else if (m[4]) out += `<span style="color:#9ec3e6">${s}</span>`;
        else if (m[5]) out += `<span style="color:#7f93a8">${s}</span>`;
        else out += `<span style="color:#7f93a8">${s}</span>`;
        last = m.index + m[0].length;
      }
      return out + esc(line.slice(last));
    })
    .join("\n");
}
void KW;

/** A floating code card (harbor panel, soft shadow). Text revealed by char count. */
export function codeCard(parent, src) {
  const card = el("div", "abs", parent);
  const pre = el("pre", "", card);
  S(card, { background: "#101f30", borderRadius: "22px", boxShadow: "0 40px 90px rgba(0,0,0,.5), 0 0 0 1px rgba(238,241,244,.12)", padding: "0.9em 1.1em", fontFamily: "var(--mono)", color: "#eef1f4" });
  S(pre, { margin: 0, fontFamily: "inherit", fontWeight: 500, lineHeight: 1.42, whiteSpace: "pre" });
  return { card, pre, src, lastN: -1 };
}
export function drawCode(cc, n, caret = true) {
  n = Math.max(0, Math.min(cc.src.length, Math.floor(n)));
  if (cc.lastN === n) return;
  cc.lastN = n;
  const done = n >= cc.src.length;
  cc.pre.innerHTML = hl(cc.src.slice(0, n)) + (caret && !done ? `<span style="display:inline-block;width:0.55em;height:1.05em;vertical-align:-0.15em;background:#ef7d3e"></span>` : "");
}

// ---------------------------------------------------------------- crate
/** The crate mark (site/src/assets/illustrations/iso.ts projection + IsoFrame.astro fills). */
export function crateSVG(U = 200, braces = 1) {
  const H = (U * 5.8) / 11, V = (U * 10.4) / 11;
  const P = ([x, y, z]) => [(x - y) * U, (x + y) * H - z * V];
  const poly = (pts) => "M" + pts.map(P).map(([a, b]) => `${a.toFixed(1)} ${b.toFixed(1)}`).join("L") + "Z";
  const top = poly([[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]]);
  const left = poly([[0, 1, 0], [1, 1, 0], [1, 1, 1], [0, 1, 1]]);
  const right = poly([[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]]);
  // braces: from the front-top corner diagonally across each face
  const c = P([1, 1, 1]);
  const ln = (a, b) => `M${P(a)[0].toFixed(1)} ${P(a)[1].toFixed(1)}L${P(b)[0].toFixed(1)} ${P(b)[1].toFixed(1)}`;
  return { top, left, right, br: [ln([1, 1, 1], [0, 1, 0]), ln([1, 1, 1], [1, 0, 0]), ln([1, 1, 1], [0, 0, 1])], c, P };
}
