// A real dashboard capture as a camera plane. Loads capture/shots/<name>.png
// (DPR 4) and its <name>.boxes.json (element rects measured by
// capture/capture.mjs), so scenes can say "frame the Update rule input"
// and "pin a callout to the Geo point selector" in plane pixels.

import { el, S } from "./kit.js";
import { css } from "./cam.js";

const cache = new Map();
export async function loadShot(name) {
  if (cache.has(name)) return cache.get(name);
  const meta = await (await fetch(`capture/shots/${name}.boxes.json`)).json();
  cache.set(name, meta);
  return meta;
}

export class UIPlane {
  constructor(parent, name, meta) {
    this.name = name;
    this.meta = meta;
    this.dpr = meta.dpr;
    this.w = meta.width * meta.dpr;
    this.h = meta.height * meta.dpr;
    this.root = el("div", "plane", parent);
    this.img = el("img", "", this.root);
    this.img.src = `capture/shots/${name}.png`;
    S(this.img, { width: this.w + "px", height: this.h + "px" });
    S(this.root, { width: this.w + "px", height: this.h + "px", boxShadow: "0 60px 160px rgba(0,0,0,.55)", borderRadius: "24px", overflow: "hidden", background: "#0a0a0a" });
    this.overlay = el("div", "abs", this.root);
  }
  decode() {
    return this.img.decode().catch(() => {});
  }
  /**
   * Find a measured box. `q` is a string (exact text match first, then
   * substring) or a predicate. `n` picks the n-th match. Returns plane px.
   */
  box(q, { kind, n = 0, pad = 0 } = {}) {
    const bs = this.meta.boxes.filter((b) => !kind || b.kind === kind);
    let hits = typeof q === "function" ? bs.filter(q) : bs.filter((b) => b.text === q);
    if (!hits.length && typeof q === "string") hits = bs.filter((b) => b.text.includes(q));
    const b = hits[n];
    if (!b) throw new Error(`${this.name}: no box for ${q}`);
    const d = this.dpr;
    return { x: (b.x - pad) * d, y: (b.y - pad) * d, w: (b.w + 2 * pad) * d, h: (b.h + 2 * pad) * d };
  }
  /** Union of several boxes (plane px). */
  union(...rs) {
    const x0 = Math.min(...rs.map((r) => r.x)), y0 = Math.min(...rs.map((r) => r.y));
    const x1 = Math.max(...rs.map((r) => r.x + r.w)), y1 = Math.max(...rs.map((r) => r.y + r.h));
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }
  /** CSS-px rect → plane px. */
  rect(x, y, w, h) {
    const d = this.dpr;
    return { x: x * d, y: y * d, w: w * d, h: h * d };
  }
  set(m) {
    S(this.root, { transform: css(m) });
    this.m = m;
  }
  /** A rectangle on the plane (masks, highlight rings), in plane px. */
  rectEl(r, style = {}) {
    const e = el("div", "abs", this.overlay);
    S(e, { left: r.x + "px", top: r.y + "px", width: r.w + "px", height: r.h + "px", ...style });
    return e;
  }
}
