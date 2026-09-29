// Chapter 6 — Superpowers (36–48 s). docs/shotlist.md §6. The densest
// chapter: four live panels, each a full-bleed solo for one bar, then all
// four at once in a 2×2 wall, then a fly-over into the hard cut at 48.
//
// Panels are RECREATIONS in examples/team-board's real dark tokens and
// fonts, fed by real data: the film's seeded places/posts (capture/seed.sh),
// team-board's seed (board columns, card titles, users), real distances
// (haversine from the query point; order verified against a live server's
// `sort=geoDistance(location.lon, location.lat, 106.8045, -6.2385)`), and
// real SDK calls. Search and geo sort ship in 0.4.0; presence and
// notifications are from feat/notifications-realtime and are labelled
// "Coming in v0.5" on screen.

import { el, S, place, show, px, heavy, snappy, soft, chrome, prog, statement, drawStatement, cursor, drawCursor, html, text, easeIn, lerp } from "../lib/kit.js";
import { clamp, track, mulberry32 } from "../lib/motion.js";
import { addCue } from "../lib/timeline.js";

const C = { bg: "#16181d", panel: "#1b1e25", line: "#2b2f38", paper: "#f6f4ef", dim: "rgba(246,244,239,.5)", crate: "#f07440", manifest: "#6c8fc7", moss: "#3e8b63" };

// ------------------------------------------------------------------ data
const DOCS = [
  { t: "Kopi Kenangan Senopati", b: "Third-wave coffee near the office, quiet before 9am.", c: "places" },
  { t: "Blok M Co-working Loft", b: "Fast wifi, standing desks, a rooftop for calls.", c: "places" },
  { t: "Pasar Santa Vinyl Corner", b: "A crate of second-hand vinyl worth digging through.", c: "places" },
  { t: "Warung Tekko", b: "Grilled fish and sambal matah, worth a detour.", c: "places" },
  { t: "Why we moved off three services onto one binary", b: "Collections, auth and realtime used to be three vendors. Now it is one crate.", c: "posts" },
  { t: "Shipping the places search feature", b: "Full-text search across name and description, ranked, in one query parameter.", c: "posts" },
];
const QUERY = "coffee";
const Q_T = [36.25, 36.375, 36.5, 36.625, 36.75, 36.875]; // one letter per 16th
const QUERY2 = "vinyl";
const Q2_T = [44.5, 44.625, 44.75, 44.875, 45.0];
const words = (s) => s.toLowerCase().match(/[a-z0-9-]+/g) || [];
function score(doc, q) {
  if (!q) return 0;
  let sc = 0;
  for (const w of words(doc.t)) if (w.startsWith(q) || w.split("-").some((p) => p.startsWith(q))) sc += 3;
  for (const w of words(doc.b)) if (w.startsWith(q)) sc += 1;
  return sc;
}
function ranking(q) {
  const s = DOCS.map((d, i) => ({ i, s: score(d, q) }));
  s.sort((a, b) => b.s - a.s || a.i - b.i);
  const rank = [];
  s.forEach((e, r) => (rank[e.i] = r));
  return { rank, score: DOCS.map((d) => score(d, q)) };
}
function markup(str, q) {
  const esc = (x) => x.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  if (!q) return esc(str);
  return str.replace(/[A-Za-z0-9-]+/g, (w) => {
    const lw = w.toLowerCase();
    const parts = lw.split("-");
    if (lw.startsWith(q)) return `<mark>${esc(w.slice(0, q.length))}</mark>${esc(w.slice(q.length))}`;
    if (parts.length > 1) {
      let off = 0;
      for (const p of parts) {
        if (p.startsWith(q)) return esc(w.slice(0, off)) + `<mark>${esc(w.slice(off, off + q.length))}</mark>` + esc(w.slice(off + q.length));
        off += p.length + 1;
      }
    }
    return esc(w);
  });
}

const ME = { lat: -6.2385, lon: 106.8045 };
const PLACES = [
  { n: "Kopi Kenangan Senopati", lat: -6.2407, lon: 106.7996, d: 594 },
  { n: "Blok M Co-working Loft", lat: -6.244, lon: 106.8022, d: 662 },
  { n: "Pasar Santa Vinyl Corner", lat: -6.2412, lon: 106.8107, d: 748 },
  { n: "Warung Tekko", lat: -6.2297, lon: 106.809, d: 1098 },
];
const PIN_T = [38.5, 38.75, 39.0, 39.25];

const COLS = ["Backlog", "In Progress", "Review", "Done"];
const CARDS = [
  [0, "Design the onboarding flow"], [0, "Set up CI for the marketing site"],
  [1, "Fix flaky checkout test"], [1, "Add CSV export to the reports page"],
  [2, "Dark mode for the dashboard"], [3, "Migrate auth to the new session model"],
];
const PEOPLE = [
  { n: "Alice Chen", c: C.manifest, seed: 11, t: 40.0 },
  { n: "Bob Diaz", c: C.moss, seed: 23, t: 40.25 },
  { n: "Carol Nkemelu", c: C.crate, seed: 37, t: 40.5 },
  { n: "You", c: C.paper, seed: 51, t: 40.75 },
];
const NOTES = [
  "Carol assigned you “Fix flaky checkout test”",
  "Bob moved “Dark mode for the dashboard” to Done",
  "Alice commented on “Design the onboarding flow”",
  "Overdue: Set up CI for the marketing site",
  "Bob mentioned you on “Add CSV export to the reports page”",
  "Carol completed “Migrate auth to the new session model”",
];
const NOTE_T = [42.0, 42.125, 42.25, 42.375, 42.5, 42.625].map((t) => t + 0.125);
const MARK_READ = 43.5;

// ------------------------------------------------------------------ cues
addCue(36.0, "whoosh", { dur: 0.4, gain: 0.6 });
Q_T.forEach((t) => addCue(t, "keys", { n: 1, dur: 0.05 }));
[37.0, 37.5].forEach((t) => addCue(t, "tick", { note: 88 }));
addCue(38.0, "whoosh", { dur: 0.35, gain: 0.5 });
addCue(38.0, "riser", { dur: 0.5 });
PIN_T.forEach((t, i) => addCue(t, "pin", { note: 72 + [0, 3, 7, 10][i] }));
addCue(40.0, "whoosh", { dur: 0.35, gain: 0.5 });
PEOPLE.forEach((p, i) => addCue(p.t, "pop", { note: 67 + i * 4 }));
addCue(41.0, "click", {});
addCue(41.5, "pop", { note: 79 });
addCue(42.0, "whoosh", { dur: 0.35, gain: 0.5 });
NOTE_T.forEach((t, i) => addCue(t, "tick", { note: 84 + i * 2 }));
addCue(MARK_READ, "click", {});
addCue(MARK_READ + 0.02, "confirm", {});
addCue(44.0, "whoosh", { dur: 0.6, gain: 0.8 });
Q2_T.forEach((t) => addCue(t, "keys", { n: 1, dur: 0.05 }));
[45.0, 45.25, 45.5].forEach((t, i) => addCue(t, "tick", { note: 88 + i * 3 }));
addCue(46.0, "riser", { dur: 1.9 });
addCue(47.0, "whoosh", { dur: 1.0, gain: 0.9 });

// ------------------------------------------------------------------ panels
const PANEL_CSS = `
  .sp { position:absolute; left:0; top:0; overflow:hidden; background:${C.bg}; color:${C.paper}; font-family:Inter, sans-serif; transform-origin:0 0; }
  .sp mark { background: rgba(240,116,64,.28); color:#fff; border-radius:4px; padding:0 2px; }
  .sp .chip { position:absolute; font-family:RHMono, monospace; font-weight:600; color:${C.paper}; background:${C.panel}; border:2px solid ${C.line}; border-radius:14px; padding:.35em .7em; white-space:nowrap; }
  .sp .soon { font-family:Inter, sans-serif; font-weight:600; color:${C.bg}; background:${C.crate}; border-radius:999px; padding:.2em .7em; white-space:nowrap; position:absolute; }
`;

function mkPanel(root, W, H) {
  const p = el("div", "sp", root);
  S(p, { width: W + "px", height: H + "px" });
  return p;
}

// Search -------------------------------------------------------------
function buildSearch(root, W, H, portrait) {
  const p = mkPanel(root, W, H);
  const u = W / 1920;
  const pad = (portrait ? 64 : 110) * u * (portrait ? 1.78 : 1);
  const fieldY = H * (portrait ? 0.2 : 0.25), fieldH = (portrait ? 150 : 140) * (portrait ? W / 1080 : u);
  const field = el("div", "abs", p);
  S(field, { left: px(pad), top: px(fieldY), width: px(W - 2 * pad), height: px(fieldH), background: C.panel, border: `3px solid ${C.crate}`, borderRadius: px(fieldH * 0.22), display: "flex", alignItems: "center", gap: px(fieldH * 0.25), padding: `0 ${px(fieldH * 0.35)}` });
  el("div", "", field, `<svg width="${fieldH * 0.4}" height="${fieldH * 0.4}" viewBox="0 0 24 24" fill="none" stroke="${C.dim}" stroke-width="2.4" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/></svg>`);
  const q = el("div", "nowrap", field);
  S(q, { fontSize: px(fieldH * 0.5), fontWeight: 500, letterSpacing: "-0.01em" });
  const count = el("div", "abs nowrap", p);
  S(count, { fontFamily: "RHMono, monospace", fontSize: px(fieldH * 0.24), color: C.dim, left: px(pad), top: px(fieldY + fieldH + fieldH * 0.22) });
  const listY = fieldY + fieldH * 1.75;
  const rowH = (portrait ? 150 * (W / 1080) : 138 * u);
  const rows = DOCS.map((d) => {
    const r = el("div", "abs", p);
    S(r, { left: px(pad), width: px(W - 2 * pad), height: px(rowH - 10 * u), borderBottom: `2px solid ${C.line}`, overflow: "hidden" });
    const tag = el("div", "abs nowrap", r, d.c);
    S(tag, { left: "auto", right: "0px", top: px(rowH * 0.14), fontFamily: "RHMono, monospace", fontSize: px(rowH * 0.2), color: d.c === "posts" ? C.manifest : C.crate, border: `2px solid ${C.line}`, borderRadius: px(8), padding: "0.05em 0.45em" });
    const t = el("div", "abs nowrap", r);
    S(t, { left: "0px", top: px(rowH * 0.1), fontSize: px(rowH * 0.34), fontWeight: 600, letterSpacing: "-0.01em", maxWidth: px(W - 2 * pad - rowH * 1.6), overflow: "hidden", textOverflow: "ellipsis" });
    const b = el("div", "abs nowrap", r);
    S(b, { left: "0px", top: px(rowH * 0.54), fontSize: px(rowH * 0.23), color: C.dim, maxWidth: px(W - 2 * pad), overflow: "hidden", textOverflow: "ellipsis" });
    return { r, t, b };
  });
  const chip = el("div", "chip", p, `cb.collection("places").list({ search: "coffee" })`);
  S(chip, { fontSize: px((portrait ? 30 : 38) * (portrait ? W / 1080 : u)), left: "auto", right: px(pad), top: px(fieldY - fieldH * 0.78) });
  if (portrait) S(chip, { right: "auto", left: px(pad) });
  return { p, q, rows, count, listY, rowH, chip, u };
}
function drawSearch(s, t, typedAt, query, t0) {
  const n = typedAt.filter((x) => t >= x).length;
  const q = query.slice(0, n);
  const caret = Math.floor(t * 4) % 2 === 0 ? `<span style="display:inline-block;width:.06em;height:1em;vertical-align:-.12em;background:${C.crate};margin-left:.04em"></span>` : "";
  html(s.q, (q || `<span style="color:${C.dim}">Search places and posts</span>`) + caret);
  // rank springs: one track() per row over every keystroke
  const steps = [[t0 - 10, ""], ...typedAt.map((x, i) => [x, query.slice(0, i + 1)])];
  const rk = steps.map(([x, qq]) => [x, ranking(qq)]);
  const cur = ranking(q);
  const hits = cur.score.filter((x) => x > 0).length;
  text(s.count, q ? `${hits} result${hits === 1 ? "" : "s"}` : "");
  s.rows.forEach((row, i) => {
    const y = track(t, rk.map(([x, r]) => [x, s.listY + r.rank[i] * s.rowH]), 320, 30);
    S(row.r, { top: px(y), opacity: String(q && cur.score[i] === 0 ? 0.28 : 1) });
    html(row.t, markup(DOCS[i].t, q));
    html(row.b, markup(DOCS[i].b, q));
  });
}

// Map ----------------------------------------------------------------
function buildMap(root, W, H, portrait) {
  const p = mkPanel(root, W, H);
  const U = portrait ? W / 1080 : W / 1920;
  // projection: metres → px around ME
  const mapCx = portrait ? W * 0.5 : W * 0.3, mapCy = portrait ? H * 0.47 : H * 0.66;
  const pxPerM = (portrait ? 0.44 : 0.38) * (portrait ? W / 1080 : W / 1920);
  const proj = (lat, lon) => [mapCx + (lon - ME.lon) * 110574 * Math.cos((ME.lat * Math.PI) / 180) * pxPerM, mapCy - (lat - ME.lat) * 110574 * pxPerM];
  // procedural dark street map (seeded; no labels, no third-party map style)
  const rnd = mulberry32(90210);
  let streets = "";
  for (let i = 0; i < 26; i++) {
    const vertical = i % 2 === 0;
    const off = (rnd() - 0.5) * (vertical ? W : H) * 1.3;
    const ang = (vertical ? 78 : -12) + (rnd() - 0.5) * 16;
    const a = (ang * Math.PI) / 180;
    const cx = W / 2 + (vertical ? off : 0), cy = H / 2 + (vertical ? 0 : off);
    const L2 = Math.max(W, H) * 1.5;
    const w = rnd() < 0.2 ? 14 : rnd() < 0.5 ? 7 : 4;
    streets += `<path d="M${(cx - Math.cos(a) * L2).toFixed(0)} ${(cy - Math.sin(a) * L2).toFixed(0)}L${(cx + Math.cos(a) * L2).toFixed(0)} ${(cy + Math.sin(a) * L2).toFixed(0)}" stroke="${w > 10 ? "#2e333d" : "#23272f"}" stroke-width="${(w * U * 1.6).toFixed(1)}" stroke-linecap="round"/>`;
  }
  let blocks = "";
  for (let i = 0; i < 70; i++) {
    const x = rnd() * W, y = rnd() * H, w = (40 + rnd() * 120) * U, h = (30 + rnd() * 90) * U;
    blocks += `<rect x="${x.toFixed(0)}" y="${y.toFixed(0)}" width="${w.toFixed(0)}" height="${h.toFixed(0)}" rx="${(6 * U).toFixed(1)}" fill="#1c1f26"/>`;
  }
  const park = `<path d="M${(W * 0.62).toFixed(0)} ${(H * 0.18).toFixed(0)} q${(120 * U).toFixed(0)} ${(-40 * U).toFixed(0)} ${(260 * U).toFixed(0)} ${(30 * U).toFixed(0)} t${(60 * U).toFixed(0)} ${(180 * U).toFixed(0)} q${(-160 * U).toFixed(0)} ${(80 * U).toFixed(0)} ${(-300 * U).toFixed(0)} ${(-20 * U).toFixed(0)}z" fill="#1d2a24"/>`;
  el("div", "abs", p, `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">${blocks}${park}${streets}</svg>`);
  const sweep = el("div", "abs", p);
  const me = proj(ME.lat, ME.lon);
  const R = 1150 * pxPerM;
  S(sweep, { left: px(me[0] - R), top: px(me[1] - R), width: px(2 * R), height: px(2 * R), borderRadius: "50%", border: `${(4 * U).toFixed(1)}px solid ${C.crate}`, background: "rgba(240,116,64,.07)" });
  const you = el("div", "abs", p);
  S(you, { left: px(me[0] - 18 * U), top: px(me[1] - 18 * U), width: px(36 * U), height: px(36 * U), borderRadius: "50%", background: C.manifest, border: `${(6 * U).toFixed(1)}px solid ${C.paper}` });
  const youPulse = el("div", "abs", p);
  S(youPulse, { left: px(me[0] - 60 * U), top: px(me[1] - 60 * U), width: px(120 * U), height: px(120 * U), borderRadius: "50%", border: `${(4 * U).toFixed(1)}px solid ${C.manifest}` });
  const pins = PLACES.map((pl, i) => {
    const [x, y] = proj(pl.lat, pl.lon);
    const g = el("div", "abs", p);
    const pin = el("div", "abs", g, `<svg width="${76 * U}" height="${100 * U}" viewBox="0 0 28 37"><path d="M14 36s12-12.6 12-21.5A12 12 0 0 0 2 14.5C2 23.4 14 36 14 36z" fill="${C.crate}" stroke="${C.bg}" stroke-width="1.6"/><text x="14" y="19.2" text-anchor="middle" font-family="Inter" font-weight="800" font-size="13" fill="${C.bg}">${i + 1}</text></svg>`);
    S(pin, { left: px(-38 * U), top: px(-100 * U) });
    return { g, x, y };
  });
  // ranked list
  const list = el("div", "abs", p);
  const lx = portrait ? W * 0.06 : W * 0.6, ly = portrait ? H * 0.7 : H * 0.36, lw = portrait ? W * 0.88 : W * 0.36;
  S(list, { left: px(lx), top: px(ly), width: px(lw) });
  const rowH = (portrait ? 118 : 128) * U;
  const rows = PLACES.map((pl, i) => {
    const r = el("div", "abs", list);
    S(r, { left: "0px", top: px(i * rowH), width: px(lw), height: px(rowH - 12 * U), background: C.panel, border: `2px solid ${C.line}`, borderRadius: px(14 * U), display: "flex", alignItems: "center", justifyContent: "space-between", padding: `0 ${px(22 * U)}` });
    el("div", "nowrap", r, `<b style="color:${C.crate};margin-right:.5em">${i + 1}</b>${pl.n}`).style.cssText = `font-size:${px(36 * U)};font-weight:600;overflow:hidden;text-overflow:ellipsis;max-width:${px(lw * 0.7)}`;
    el("div", "nowrap", r, `${pl.d} m`).style.cssText = `font-family:RHMono,monospace;font-size:${px(36 * U)};font-weight:600;color:${C.paper}`;
    return r;
  });
  const chip = el("div", "chip", p, `sort: "geoDistance(location.lon, location.lat, 106.8045, -6.2385)"`);
  S(chip, { fontSize: px(32 * U), left: px(W * (portrait ? 0.06 : 0.05)), top: px(portrait ? H * 0.64 : H * 0.88) });
  return { p, sweep, R, you, youPulse, pins, rows, chip, U };
}
function drawMap(s, t, base) {
  const sw = soft(t - base);
  S(s.sweep, { transform: `scale(${(0.02 + 0.98 * sw).toFixed(4)})` });
  const pulse = ((t - base) % 1 + 1) % 1;
  S(s.youPulse, { transform: `scale(${(0.4 + pulse * 1.4).toFixed(3)})`, opacity: (1 - pulse).toFixed(3) });
  s.pins.forEach((pn, i) => {
    const t0 = base + 0.5 + i * 0.25;
    const on = t >= t0;
    show(pn.g, on);
    if (!on) return;
    const d = t - t0;
    const fall = d < 0.18 ? easeIn(d / 0.18) : 1;
    const sq = d >= 0.18 ? 1 - 0.18 * Math.exp(-(d - 0.18) * 14) * Math.cos((d - 0.18) * 30) : 1;
    place(pn.g, pn.x, pn.y - (1 - fall) * 260 * s.U, ` scale(${(1 / sq).toFixed(3)},${sq.toFixed(3)})`);
  });
  s.rows.forEach((r, i) => {
    const p = heavy(t - (base + 0.5 + i * 0.25));
    S(r, { transform: `translate3d(${((1 - p) * 140).toFixed(1)}%,0,0)` });
  });
}

// Presence board -------------------------------------------------------
function buildBoard(root, W, H, portrait) {
  const p = mkPanel(root, W, H);
  const U = portrait ? W / 1080 : W / 1920;
  const pad = 60 * U;
  const top = portrait ? H * 0.3 : H * 0.4;
  const head = el("div", "abs nowrap", p, "Acme Inc");
  S(head, { left: px(pad), top: px(top - 100 * U), fontFamily: "Archivo, sans-serif", fontWeight: 700, fontSize: px(52 * U) });
  const stack = el("div", "abs", p);
  S(stack, { left: "auto", right: px(pad), top: px(top - 104 * U), height: px(70 * U), display: "flex", alignItems: "center" });
  const avatars = PEOPLE.map((pp, i) => {
    const a = el("div", "", stack, pp.n === "You" ? "Y" : pp.n.split(" ").map((x) => x[0]).join(""));
    S(a, { width: px(70 * U), height: px(70 * U), borderRadius: "50%", background: pp.c, color: C.bg, fontWeight: 700, fontSize: px(26 * U), display: "flex", alignItems: "center", justifyContent: "center", border: `${px(5 * U)} solid ${C.bg}`, marginLeft: i ? px(-18 * U) : "0" });
    return a;
  });
  const here = el("div", "nowrap", stack);
  S(here, { marginLeft: px(20 * U), fontSize: px(30 * U), color: C.dim, fontWeight: 500 });
  const colW = (W - pad * 2 - 24 * U * 3) / 4;
  const colEls = COLS.map((n, i) => {
    const c = el("div", "abs", p);
    S(c, { left: px(pad + i * (colW + 24 * U)), top: px(top), width: px(colW), height: px(H - top - pad), background: C.panel, border: `2px solid ${C.line}`, borderRadius: px(18 * U) });
    const h = el("div", "abs nowrap", c, `${n}`);
    S(h, { left: px(22 * U), top: px(20 * U), fontSize: px(30 * U), fontWeight: 600, color: C.dim });
    return c;
  });
  const cardH = (portrait ? 190 : 150) * U;
  const cardEls = CARDS.map(([ci, title], k) => {
    const e = el("div", "abs", p, title);
    S(e, { width: px(colW - 36 * U), height: px(cardH), background: C.bg, border: `2px solid ${C.line}`, borderRadius: px(12 * U), padding: px(20 * U), fontSize: px((portrait ? 26 : 29) * U), fontWeight: 500, lineHeight: 1.25 });
    return e;
  });
  const slot = (ci, row) => [pad + ci * (colW + 24 * U) + 18 * U, top + 80 * U + row * (cardH + 16 * U)];
  const curs = PEOPLE.map((pp) => {
    const g = el("div", "abs", p);
    el("div", "abs", g, `<svg width="${40 * U}" height="${50 * U}" viewBox="0 0 22 27"><path d="M2 1.5v20.2l5.1-4.9 3.3 7.7 3.4-1.5-3.3-7.5h7.1z" fill="${pp.c}" stroke="${C.bg}" stroke-width="1.6" stroke-linejoin="round"/></svg>`);
    const lab = el("div", "abs nowrap", g, pp.n.split(" ")[0]);
    S(lab, { left: px(34 * U), top: px(40 * U), background: pp.c, color: C.bg, fontWeight: 700, fontSize: px(28 * U), padding: ".12em .5em", borderRadius: px(10 * U) });
    return g;
  });
  const chip = el("div", "chip", p, `usePresence(cb, "board:acme", { cursor })`);
  S(chip, { fontSize: px(32 * U), left: "auto", right: px(pad), top: px(portrait ? top - 210 * U : H * 0.17) });
  return { p, avatars, here, cardEls, slot, curs, colW, cardH, U, W, H, chip };
}
function cursorPath(seed, t, W, H) {
  const r = mulberry32(seed);
  const a = [r(), r(), r(), r(), r(), r()].map((v) => v * Math.PI * 2);
  const x = W * (0.5 + 0.32 * Math.sin(t * 1.3 + a[0]) + 0.1 * Math.sin(t * 3.1 + a[1]));
  const y = H * (0.6 + 0.22 * Math.sin(t * 1.1 + a[2]) + 0.08 * Math.sin(t * 2.7 + a[3]));
  return [x, y];
}
function drawBoard(s, t, base, dragOn = true) {
  const n = PEOPLE.filter((pp) => t >= pp.t - (40 - base)).length;
  s.avatars.forEach((a, i) => S(a, { transform: `scale(${chrome(t - (PEOPLE[i].t - (40 - base))).toFixed(3)})` }));
  text(s.here, n ? `${n} here` : "");
  // Bob (index 1) drags "Dark mode" (card 4) from Review to Done at base+1.0 → +1.5
  const from = s.slot(2, 0), to = s.slot(3, 1);
  const dragP = dragOn ? soft(t - (base + 1.0)) : 0;
  const lifted = dragOn && t >= base + 0.95 && t < base + 1.6;
  CARDS.forEach(([ci], k) => {
    const row = CARDS.slice(0, k).filter(([c]) => c === ci).length;
    let [x, y] = s.slot(ci, row);
    if (k === 4) {
      x = from[0] + (to[0] - from[0]) * dragP;
      y = from[1] + (to[1] - from[1]) * dragP - (lifted ? 14 * s.U : 0);
    }
    place(s.cardEls[k], x, y, k === 4 && lifted ? ` rotate(${(-2 * Math.sin(dragP * Math.PI)).toFixed(2)}deg)` : "");
    S(s.cardEls[k], { boxShadow: k === 4 && lifted ? `${px(6 * s.U)} ${px(6 * s.U)} 0 rgba(0,0,0,.45)` : "none", zIndex: k === 4 ? "5" : "1", borderColor: k === 4 && lifted ? C.moss : C.line });
  });
  s.curs.forEach((g, i) => {
    const pp = PEOPLE[i];
    const t0 = pp.t - (40 - base);
    show(g, t >= t0);
    let [x, y] = cursorPath(pp.seed, t, s.W, s.H);
    if (i === 1 && dragOn) {
      // Bob's hand is on the card while dragging
      const grab = [from[0] + s.colW * 0.5, from[1] + s.cardH * 0.5];
      const drop = [to[0] + s.colW * 0.5, to[1] + s.cardH * 0.5];
      const w = clamp((t - (base + 0.7)) / 0.3) * (1 - clamp((t - (base + 1.7)) / 0.3));
      const hx = grab[0] + (drop[0] - grab[0]) * dragP, hy = grab[1] + (drop[1] - grab[1]) * dragP;
      x = lerp(x, hx, w);
      y = lerp(y, hy, w);
    }
    const e = soft(t - t0);
    place(g, x, y + (1 - e) * 80 * s.U);
  });
}

// Notifications ----------------------------------------------------------
function buildNotes(root, W, H, portrait) {
  const p = mkPanel(root, W, H);
  const U = portrait ? W / 1080 : W / 1920;
  const bellS = (portrait ? 330 : 420) * U;
  const bx = portrait ? W * 0.5 - bellS / 2 : W * 0.08, by = portrait ? H * 0.2 : H * 0.42;
  const bell = el("div", "abs", p, `<svg width="${bellS}" height="${bellS}" viewBox="0 0 24 24" fill="none" stroke="${C.paper}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M6 8a6 6 0 1 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>`);
  S(bell, { left: px(bx), top: px(by), transformOrigin: "50% 8%" });
  const badge = el("div", "abs", p);
  const bs = bellS * 0.42;
  S(badge, { left: px(bx + bellS * 0.62), top: px(by - bs * 0.1), width: px(bs), height: px(bs), borderRadius: "50%", background: C.crate, color: "#fff", fontWeight: 700, fontSize: px(bs * 0.58), display: "flex", alignItems: "center", justifyContent: "center", fontFamily: "Inter, sans-serif", border: `${px(8 * U)} solid ${C.bg}` });
  const lx = portrait ? W * 0.06 : W * 0.38, ly = portrait ? H * 0.47 : H * 0.36, lw = portrait ? W * 0.9 : W * 0.58;
  const menu = el("div", "abs", p);
  S(menu, { left: px(lx), top: px(ly), width: px(lw), height: px((portrait ? H * 0.5 : H * 0.62)), background: C.panel, border: `2px solid ${C.line}`, borderRadius: px(22 * U), overflow: "hidden" });
  const mh = el("div", "abs nowrap", menu);
  S(mh, { left: px(28 * U), right: px(28 * U), top: px(22 * U), height: px(60 * U), display: "flex", alignItems: "center", justifyContent: "space-between", fontSize: px(30 * U), fontWeight: 600 });
  mh.innerHTML = `<span>Notifications</span><span data-k="mar" style="color:${C.manifest};font-weight:600">Mark all read</span>`;
  const rowH = (portrait ? 124 : 104) * U;
  const rows = NOTES.map((n, i) => {
    const r = el("div", "abs", menu);
    S(r, { left: px(16 * U), right: px(16 * U), height: px(rowH - 8 * U), borderRadius: px(12 * U), padding: `${px(14 * U)} ${px(20 * U)} 0 ${px(52 * U)}` });
    const dot = el("div", "abs", r);
    S(dot, { left: px(20 * U), top: px(26 * U), width: px(16 * U), height: px(16 * U), borderRadius: "50%", background: C.crate });
    const tx = el("div", "nowrap", r, n);
    S(tx, { fontSize: px((portrait ? 32 : 36) * U), fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", maxWidth: px(lw - 110 * U) });
    const tm = el("div", "nowrap", r, i === 0 ? "just now" : `${i * 2} min ago`);
    S(tm, { fontSize: px(22 * U), color: C.dim, marginTop: px(6 * U) });
    return { r, dot, tx };
  });
  const chip = el("div", "chip", p, `cb.notifications.markAllRead()`);
  S(chip, { fontSize: px(32 * U), left: "auto", right: px(W * 0.04), top: px(portrait ? H * 0.47 - 100 * U : H * 0.27) });
  const mar = mh.querySelector('[data-k="mar"]');
  return { p, bell, badge, rows, rowH, menu, mar, U, lx, ly, lw, chip };
}
function drawNotes(s, t, base, markAt) {
  const times = NOTE_T.map((x) => x - 42 + base);
  const n = times.filter((x) => t >= x).length;
  const read = markAt !== null && t >= markAt;
  const count = read ? 0 : n;
  text(s.badge, String(count));
  // badge: snaps on each increment (chromeSnap), vanishes on mark-read
  const lastInc = [...times].reverse().find((x) => t >= x);
  const bump = lastInc !== undefined ? chrome(t - lastInc) : 0;
  const out = read ? 1 - chrome(t - markAt) : 1;
  S(s.badge, { transform: `scale(${(count || (read && out > 0.02) ? (0.8 + 0.2 * bump) * Math.max(0, out) : 0).toFixed(3)})` });
  // bell rings on each increment (damped wobble)
  let rot = 0;
  for (const x of times) if (t >= x) rot += 14 * Math.exp(-(t - x) * 7) * Math.sin((t - x) * 28);
  S(s.bell, { transform: `rotate(${rot.toFixed(2)}deg)` });
  s.rows.forEach((r, i) => {
    const t0 = times[times.length - 1 - i]; // newest on top: row 0 appears last
    const shown = NOTES.length - n; // rows below the newest
    void shown;
    const k = i; // rows cascade in from top
    const p = heavy(t - times[k]);
    show(r.r, t >= times[k]);
    S(r.r, { top: px(90 * s.U + k * s.rowH), transform: `translate3d(${((1 - p) * 60).toFixed(1)}%,0,0)`, background: read ? "transparent" : "rgba(240,116,64,.07)" });
    S(r.dot, { transform: `scale(${read ? (1 - chrome(t - markAt)).toFixed(3) : "1"})` });
    S(r.tx, { color: read ? C.dim : C.paper, fontWeight: read ? "500" : "600" });
    void t0;
  });
}

// ------------------------------------------------------------------ scene
export default [
  {
    id: "superpowers",
    from: 36,
    to: 48,
    z: 3,
    build(root, L) {
      S(root, { background: C.bg });
      el("style", "", root, PANEL_CSS);
      const P = L.portrait;
      const W = L.W, H = L.H;
      const stage = el("div", "abs", root);
      S(stage, { width: W + "px", height: H + "px", transformOrigin: "50% 50%" });
      const panels = {
        search: buildSearch(stage, W, H, P),
        map: buildMap(stage, W, H, P),
        board: buildBoard(stage, W, H, P),
        notes: buildNotes(stage, W, H, P),
      };
      const cur = cursor(root);
      const st = {
        search: statement(root, ["Full-text search."], "disp"),
        map: statement(root, ["Sort by distance."], "disp"),
        board: statement(root, ["Live presence."], "disp"),
        notes: statement(root, ["Notifications."], "disp"),
        wall: statement(root, P ? ["All of it,", "one binary."] : ["All of it, one binary."], "disp"),
      };
      const soon = el("div", "abs nowrap", root, "Coming in v0.5");
      S(soon, { fontFamily: "Inter, sans-serif", fontWeight: 700, color: C.bg, background: C.crate, borderRadius: "999px", padding: "0.18em 0.7em 0.22em", fontSize: px((P ? 40 : 44) * L.u) });
      const plate = el("div", "abs", root);
      S(plate, { background: "#0a1622", width: L.W + "px", zIndex: "2" });
      for (const k of Object.keys(st)) S(st[k].box, { zIndex: "3" });
      return { stage, panels, cur, st, soon, plate };
    },
    draw(t, s, L) {
      const P = L.portrait;
      const W = L.W, H = L.H;
      const order = ["search", "map", "board", "notes"];
      const solo = t < 44 ? order[Math.min(3, Math.floor((t - 36) / 2))] : null;
      // wall layout 44–48: 2×2 cells, gap, the whole wall scaled in
      const gap = 18 * L.u;
      const wallP = soft(t - 44.0);
      const cellW = (W - gap) / 2, cellH = (H - gap) / 2;
      order.forEach((k, i) => {
        const pn = s.panels[k];
        const isSolo = solo === k;
        const inWall = t >= 44.0;
        show(pn.p, isSolo || inWall);
        if (!(isSolo || inWall)) return;
        if (isSolo) {
          // solo entrance: whip in from the side the previous panel leaves toward
          const tIn = 36 + i * 2;
          const e = snappy(t - tIn);
          const dir = i % 2 ? -1 : 1;
          S(pn.p, { transform: `translate3d(${((1 - e) * W * 0.35 * dir).toFixed(1)}px,0,0) scale(${(1.06 - 0.06 * e).toFixed(4)})`, zIndex: "1" });
        } else {
          const c = i % 2, r = Math.floor(i / 2);
          const sc = lerp(i === 3 ? 1 : 0.5, 0.5 - gap / (2 * W), wallP);
          const x = lerp(0, c * (cellW + gap), wallP), y = lerp(0, r * (cellH + gap), wallP);
          const e2 = i === 3 ? 1 : heavy(t - 44.0 - (i + 1) * 0.08);
          S(pn.p, { transform: `translate3d(${px(x)},${px(y + (1 - e2) * H * 0.6)},0) scale(${sc.toFixed(4)})`, zIndex: i === 3 ? "2" : "1", borderRadius: px(28 / sc) });
        }
      });
      // panel content (live even inside the wall)
      const pn = s.panels;
      if (solo === "search" || t >= 44) {
        if (t < 44) drawSearch(pn.search, t, Q_T, QUERY, 36);
        else drawSearch(pn.search, t, Q2_T, QUERY2, 44.1);
      }
      if (solo === "map" || t >= 44) drawMap(pn.map, t, t < 44 ? 38.0 : 44.2);
      if (solo === "board" || t >= 44) drawBoard(pn.board, t, t < 44 ? 40.0 : 44.3, t < 44);
      if (solo === "notes" || t >= 44) drawNotes(pn.notes, t, t < 44 ? 42.0 : 44.6, t < 44 ? MARK_READ : null);
      // chips pop with their panel
      order.forEach((k, i) => S(pn[k].chip, { transform: `scale(${snappy(t - (36 + i * 2) - 0.5).toFixed(3)})`, transformOrigin: "0 50%" }));
      // cursor clicks "Mark all read" in the notes solo
      const mr = pn.notes.mar.getBoundingClientRect();
      drawCursor(s.cur, t, [[42.6, W * 0.9, H * 1.05], [43.3, mr.left + mr.width * 0.5, mr.top + mr.height * 0.6]], [MARK_READ], L.u * 1.2, t >= 42.6 && t < 44.0);
      // statements + v0.5 pill
      const sz = (P ? 120 : 150) * L.u;
      const o = { x: L.m, y: P ? L.H * 0.06 : L.H * 0.06, size: sz, color: C.paper };
      drawStatement(s.st.search, t, 36.0, 38.0, o);
      drawStatement(s.st.map, t, 38.0, 40.0, o);
      drawStatement(s.st.board, t, 40.0, 42.0, o);
      drawStatement(s.st.notes, t, 42.0, 44.0, o);
      const soonOn = t >= 40.25 && t < 44.0;
      show(s.soon, soonOn);
      if (soonOn) {
        place(s.soon, L.m, L.H * 0.06 + sz * 1.02, ` scale(${chrome(t - (t < 42 ? 40.25 : 42.25)).toFixed(3)})`);
        S(s.soon, { transformOrigin: "0 50%" });
      }
      // wall statement sits on a navy plate over the seam so it never covers a detail
      const wsz = (P ? 130 : 170) * L.u, wy = P ? H * 0.43 : H * 0.5 - wsz * 0.55;
      const plateOn = t >= 44.3 && t < 46.3;
      show(s.plate, plateOn);
      if (plateOn) {
        const ph = (P ? 2.2 : 1.25) * wsz * (snappy(t - 44.3) - snappy(t - 46.05));
        S(s.plate, { top: px(wy + wsz * (P ? 0.95 : 0.5) - ph / 2), height: px(Math.max(0, ph)) });
      }
      drawStatement(s.st.wall, t, 44.4, 46.2, { x: L.m, y: wy, size: wsz, color: "#ffffff" });
      // 46–48: the wall tilts back and the camera flies over it into the cut
      const fly = prog(t, 46.0, 48.0);
      const tilt = soft(t - 46.0) * 38;
      const push = Math.exp(easeIn(fly) * Math.log(3.2));
      S(s.stage, { transform: `perspective(${(1600 * L.u).toFixed(0)}px) translate3d(0,${(-easeIn(fly) * H * 0.35).toFixed(1)}px,0) rotateX(${tilt.toFixed(2)}deg) scale(${push.toFixed(4)})` });
    },
  },
];
