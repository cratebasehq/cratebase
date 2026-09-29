// Chapter 7 — Proof (48–56 s). docs/shotlist.md §7. Real numbers only,
// from benchmarks/README.md's 2026-09-04 idle-host run. Note the `search`
// workload there is paged record listing (GET .../records?page=N&perPage=30),
// so it is labelled "record list reads", not full-text search.

import { el, S, place, show, px, heavy, snappy, soft, chrome, prog, statement, drawStatement, maskLine, fitWidth, html, text } from "../lib/kit.js";
import { clamp, track } from "../lib/motion.js";
import { addCue } from "../lib/timeline.js";

const CELLS = [
  { x: "9.72×", l: "Signed-in list reads, 100 clients", a: 38768, b: 3989 },
  { x: "8.53×", l: "Wide list reads (200 rows), 100 clients", a: 16414, b: 1924 },
  { x: "1.81×", l: "Creates, 100 clients", a: 11960, b: 6606 },
];
const fmt = (n) => Math.round(n).toLocaleString("en-US");

addCue(48.0, "sub", { gain: 1.0 });
for (let i = 0; i < 16; i++) addCue(48.0 + i * 0.0625, "digit", {});
addCue(49.0, "pop", { note: 74 });
[50.0, 50.25, 50.5].forEach((t, i) => addCue(t, "pop", { note: 69 + i * 3 }));
[52.0, 52.5, 53.0, 53.5].forEach((t) => addCue(t, "click", {}));
addCue(54.0, "whoosh", { dur: 0.4, gain: 0.5 });
addCue(54.5, "keys", { n: 10, dur: 0.6 });
addCue(55.25, "confirm", {});

export default [
  {
    id: "proof",
    from: 48,
    to: 56,
    z: 3,
    build(root, L) {
      S(root, { background: "#0a1622" });
      const P = L.portrait;
      // --- 48–50: the headline number
      const big = maskLine(root, "disp", "10.47×");
      S(big.wrap, { fontVariantNumeric: "tabular-nums", paddingBottom: "0.08em" });
      const probe = el("div", "abs disp nowrap", root, "10.47×");
      S(probe, { fontVariantNumeric: "tabular-nums" });
      const bigSize = P ? fitWidth(probe, (L.W - 2 * L.m) * 0.92, 300) : fitWidth(probe, L.W * 0.66, 300);
      probe.remove();
      S(big.wrap, { fontSize: px(bigSize) });
      const cap = statement(root, P ? ["faster record list reads", "at 50 concurrent clients"] : ["faster record list reads at 50 concurrent clients"]);
      const bars = ["Cratebase", "PocketBase"].map((n, i) => {
        const g = el("div", "abs", root);
        const bar = el("div", "abs", g);
        S(bar, { background: i ? "#34495e" : "#ef7d3e", borderRadius: "0 14px 14px 0" });
        const lab = el("div", "abs nowrap", g);
        S(lab, { fontFamily: "var(--mono)", fontWeight: 600, color: i ? "#93a4b6" : "#eef1f4" });
        return { g, bar, lab, n };
      });
      const src = el("div", "abs nowrap mono", root, "benchmarks/README.md, idle-host run of 2026-09-04");
      S(src, { color: "#93a4b6", fontSize: px((P ? 30 : 34) * L.u) });
      // --- 50–52: three more cells
      const cells = CELLS.map((c) => {
        const g = el("div", "abs clip", root);
        const inner = el("div", "abs", g);
        const x = el("div", "abs nowrap disp", inner, c.x);
        const l = el("div", "abs nowrap", inner, c.l);
        S(l, { fontFamily: "var(--display)", fontWeight: 600, fontStretch: "92%", color: "#93a4b6" });
        const n = el("div", "abs nowrap mono", inner, `${fmt(c.a)} vs ${fmt(c.b)} req/s`);
        S(n, { color: "#eef1f4", fontWeight: 500 });
        return { g, inner, x, l, n };
      });
      const st22 = statement(root, P ? ["Faster in 22 of", "24 benchmark cells."] : ["Faster in 22 of 24 benchmark cells."]);
      // --- 52–54: sqlite ↔ postgres
      const env = el("div", "abs nowrap mono", root);
      S(env, { fontWeight: 600, color: "#eef1f4" });
      const res = el("pre", "abs mono", root);
      S(res, { margin: 0, color: "#93a4b6", fontWeight: 500, lineHeight: 1.45, background: "#101f30", borderRadius: "20px", padding: "0.7em 0.9em", boxShadow: "0 0 0 2px rgba(238,241,244,.1)" });
      const stDb = statement(root, P ? ["SQLite or Postgres.", "Zero code changes."] : ["SQLite or Postgres.", "Zero code changes."], "disp");
      // --- 54–56: PocketBase compatibility
      const imp = el("pre", "abs mono", root);
      S(imp, { margin: 0, color: "#eef1f4", fontWeight: 500, lineHeight: 1.45 });
      const stPb = statement(root, P ? ["PocketBase-", "wire-compatible."] : ["PocketBase-wire-", "compatible."], "disp");
      const conf = el("div", "abs nowrap", root);
      S(conf, { fontFamily: "var(--display)", fontWeight: 700, fontStretch: "92%", color: "#ef7d3e" });
      return { big, bigSize, cap, bars, src, cells, st22, env, res, stDb, imp, stPb, conf };
    },
    draw(t, s, L) {
      const P = L.portrait;
      const u = L.u;
      // ---------------- 48–50.0 number + bars
      const on1 = t < 50.0;
      show(s.big.wrap, on1);
      show(s.src, t < 52.0);
      place(s.src, L.m, P ? L.H * 0.9 : L.H * 0.92);
      s.bars.forEach((b) => show(b.g, on1));
      if (on1) {
        const ratio = 1 + 9.47 * clamp(soft(t - 48.0) * 1.0);
        const shown = t >= 49.0 ? 10.47 : Math.floor(ratio * 100) / 100;
        text(s.big.inner, `${shown.toFixed(2)}×`);
        const bx = L.m * 0.8, by = P ? L.H * 0.12 : L.H * 0.02;
        place(s.big.wrap, bx, by);
        S(s.big.inner, { transform: `translate3d(0,${((1 - heavy(t - 48.0)) * 120).toFixed(1)}%,0)` });
        // bars: full-width band below the number
        const top = P ? L.H * 0.58 : L.H * 0.66;
        const maxW = L.W - 2 * L.m;
        const bh = (P ? 110 : 110) * u;
        s.bars.forEach((b, i) => {
          const v = i ? 4774 : 49986;
          const p = soft(t - 48.0 - i * 0.12);
          const w = Math.max(6, maxW * (v / 49986) * p);
          place(b.g, L.m, top + i * (bh + 30 * u));
          S(b.bar, { width: px(w), height: px(bh) });
          S(b.lab, { fontSize: px((P ? 38 : 46) * u), top: px(bh * 0.24) });
          html(b.lab, `${b.n} <span style="opacity:.75">${fmt(v * clamp(p * 1.05))} req/s</span>`);
          S(b.lab, { left: px(i ? w + 28 * u : Math.min(w, maxW) - (P ? 0 : 0) + 28 * u), color: i ? "#93a4b6" : "#eef1f4" });
          if (!i && !P) S(b.lab, { left: px(28 * u), color: "#0a1622" });
          if (!i && P) S(b.lab, { left: px(24 * u), color: "#0a1622" });
        });
      }
      const capO = P ? { x: L.m, y: L.H * 0.12 + s.bigSize * 0.95, size: 70 * u, color: "#93a4b6" } : { x: L.m, y: L.H * 0.66 - 100 * u, size: 64 * u, color: "#93a4b6" };
      drawStatement(s.cap, t, 48.5, 50.0, capO);
      // ---------------- 50–52 more cells
      const on2 = t >= 50.0 && t < 52.0;
      s.cells.forEach((c, i) => {
        show(c.g, on2);
        if (!on2) return;
        const rowH = P ? 330 * u : 228 * u;
        const y0 = P ? L.H * 0.3 : L.H * 0.25;
        const p = heavy(t - 50.0 - i * 0.25);
        place(c.g, L.m, y0 + i * rowH);
        S(c.g, { width: px(L.W - 2 * L.m), height: px(rowH) });
        S(c.inner, { transform: `translate3d(0,${((1 - p) * rowH).toFixed(1)}px,0)` });
        const xs = (P ? 200 : 230) * u;
        S(c.x, { fontSize: px(xs), color: i < 2 ? "#eef1f4" : "#93a4b6" });
        S(c.l, { fontSize: px((P ? 50 : 62) * u), left: px(P ? 0 : 640 * u), top: px(P ? xs * 0.86 : xs * 0.08) });
        S(c.n, { fontSize: px((P ? 36 : 42) * u), left: px(P ? 0 : 640 * u), top: px(P ? xs * 0.86 + 70 * u : xs * 0.08 + 86 * u) });
      });
      drawStatement(s.st22, t, 50.0, 52.0, { x: L.m, y: P ? L.H * 0.08 : L.H * 0.09, size: (P ? 84 : 104) * u, color: "#eef1f4" });
      // ---------------- 52–54 sqlite ↔ postgres (flips on each beat, result unchanged)
      const on3 = t >= 52.0 && t < 54.0;
      show(s.env, on3);
      show(s.res, on3);
      if (on3) {
        const pg = Math.floor((t - 52.0) / 0.5) % 2 === 1;
        const flip = snappy(t - (52.0 + Math.floor((t - 52.0) / 0.5) * 0.5));
        html(s.env, `DATABASE_URL=<span style="color:#ef7d3e">${pg ? "postgres://…" : "sqlite://…"}</span>`);
        S(s.env, { fontSize: px((P ? 50 : 72) * u), transform: `translate3d(${px(L.m)},${px(P ? L.H * 0.36 : L.H * 0.44)},0)`, clipPath: `inset(0 0 ${((1 - flip) * 100).toFixed(1)}% 0)` });
        html(s.res, `<span style="color:#eef1f4">cb.collection("places").list({ search: "coffee" })</span>\n\n→ Kopi Kenangan Senopati`);
        S(s.res, { fontSize: px((P ? 30 : 46) * u), transform: `translate3d(${px(L.m)},${px(P ? L.H * 0.47 : L.H * 0.58)},0) translateY(${((1 - heavy(t - 52.1)) * 40).toFixed(1)}px)` });
      }
      drawStatement(s.stDb, t, 52.0, 54.0, { x: L.m, y: P ? L.H * 0.08 : L.H * 0.07, size: (P ? 120 : 160) * u, color: "#eef1f4", lh: 0.9 });
      // ---------------- 54–56 PocketBase compatible
      const on4 = t >= 54.0;
      show(s.imp, on4);
      show(s.conf, on4);
      if (on4) {
        const src = P
          ? `import PocketBase from "pocketbase";\n\nconst pb = new PocketBase(\n  "http://127.0.0.1:8090"\n);`
          : `import PocketBase from "pocketbase";\n\nconst pb = new PocketBase("http://127.0.0.1:8090");`;
        const n = Math.floor(clamp((t - 54.5) / 0.6) * 12) / 12;
        html(s.imp, src.slice(0, Math.round(src.length * n)).replace(/"[^"]*"/g, (m) => `<span style="color:#f6b183">${m}</span>`));
        S(s.imp, { fontSize: px((P ? 40 : 52) * u), transform: `translate3d(${px(L.m)},${px(P ? L.H * 0.45 : L.H * 0.55)},0)` });
        text(s.conf, "180 of 181 SDK conformance tests pass.");
        S(s.conf, { fontSize: px((P ? 52 : 72) * u), transform: `translate3d(${px(L.m)},${px(P ? L.H * 0.66 : L.H * 0.78)},0) scale(${snappy(t - 55.25).toFixed(3)})`, transformOrigin: "0 50%" });
      }
      drawStatement(s.stPb, t, 54.0, 56.0, { x: L.m, y: P ? L.H * 0.08 : L.H * 0.1, size: (P ? 130 : 170) * u, color: "#eef1f4", lh: 0.9 });
    },
  },
];
