// Chapter 2 — One binary (8–14 s). docs/shotlist.md §2.
// A full-bleed terminal plane under a tilted camera: the real installer
// output (site/public/install.sh echo lines) and the real `cratebase dev`
// banner (crates/server/src/main.rs print_dev_banner, captured from a live
// run). On 12.0 the camera racks to the Dashboard URL, a click, and the
// real Overview capture swings in from depth.

import { el, S, place, show, px, heavy, soft, snappy, prog, lerp, statement, drawStatement, cursor, drawCursor, easeIn, easeOut } from "../lib/kit.js";
import { shotMatrix, camTrack, project, css, frame } from "../lib/cam.js";
import { UIPlane, loadShot } from "../lib/ui.js";
import { clamp, track } from "../lib/motion.js";
import { addCue } from "../lib/timeline.js";

// [appear time, text, style, typed?]. Typed lines reveal in 16th-note bursts.
const LINES = [
  [8.0, "$ curl -fsSL https://cratebase.dev/install.sh | sh", "cmd", 8.0, 9.0],
  [9.0, "Downloading Cratebase 0.4.0 for x86_64-unknown-linux-musl...", "out"],
  [9.25, "Cratebase 0.4.0 installed to ~/.local/bin/cratebase", "out"],
  [9.5, "$ cratebase dev", "cmd", 9.5, 9.875],
  [10.0, "  cratebase dev", "hi"],
  [10.125, "  ------------------------------------------------------------", "dim"],
  [10.25, "  API:            http://127.0.0.1:8090/api/", "kv"],
  [10.375, "  Dashboard:      http://127.0.0.1:8090/_/", "kv"],
  [10.5, "  Superuser:      admin@localhost / H7i8SDVOK3sln5I5uy9p  (generated; change it)", "kv"],
  [10.625, "  Types:          ./cratebase-types.d.ts", "kv"],
  [10.75, "  Mail inbox:     http://127.0.0.1:8090/api/dev/mails", "kv"],
];
const FS = 58; // plane px per terminal glyph row height basis
const LH = 1.55;

for (const [t0, txt, kind, a, b] of LINES) {
  if (kind === "cmd") {
    const bursts = Math.round((b - a) / 0.125);
    addCue(a, "keys", { n: bursts * 2, dur: b - a });
  } else addCue(t0, "tick", { note: 84 + (LINES.findIndex((l) => l[0] === t0) % 5) * 2 });
}
addCue(12.5, "click", {});
addCue(12.55, "whoosh", { dur: 0.5, gain: 0.7 });
addCue(10.0, "pop", { note: 62 });

export default [
  {
    id: "binary",
    from: 7.95,
    to: 14,
    z: 3,
    async build(root, L) {
      S(root, { background: "#070d14" });
      const planeW = 3000, planeH = 1400;
      const term = el("div", "plane", root);
      S(term, { width: planeW + "px", height: planeH + "px", background: "#070d14", fontFamily: "var(--mono)", fontSize: FS + "px", fontWeight: 500, color: "#eef1f4", padding: "80px 110px" });
      const rows = LINES.map(([, txt, kind], i) => {
        const r = el("div", "nowrap", term);
        S(r, { height: FS * LH + "px", lineHeight: FS * LH + "px", color: kind === "out" || kind === "dim" ? "#6f8499" : kind === "hi" ? "#ffffff" : "#eef1f4", fontWeight: kind === "hi" ? 700 : 500 });
        return r;
      });
      const probe = el("span", "abs nowrap", term, "0123456789");
      const glyph = probe.getBoundingClientRect().width / 10;
      probe.remove();
      const ring = el("div", "abs", term);
      S(ring, { border: "6px solid #ef7d3e", borderRadius: "18px" });
      // real Overview capture, swings in from depth on 12.5
      const meta = await loadShot("hd-overview");
      const ov = new UIPlane(root, "hd-overview", meta);
      const st1 = statement(root, L.portrait ? ["One binary."] : ["One", "binary."], "disp");
      const st2 = statement(root, L.portrait ? ["Collections, auth,", "files, realtime."] : ["Collections, auth,", "files, realtime."]);
      const cur = cursor(root);
      return { term, rows, ring, ov, st1, st2, cur, planeW, planeH, glyph };
    },
    draw(t, s, L) {
      // --- terminal text (typed commands in 16th bursts, output instant)
      s.rows.forEach((r, i) => {
        const [t0, txt, kind, a, b] = LINES[i];
        let shown = "";
        if (kind === "cmd") {
          const n = t < a ? 0 : t >= b ? txt.length : Math.round(2 + (txt.length - 2) * (Math.floor((t - a) / 0.0625) * 0.0625) / (b - a));
          shown = txt.slice(0, Math.max(t >= a ? 2 : 0, n));
          const caret = t >= a && t < (LINES[i + 1]?.[0] ?? 99) && Math.floor(t * 4) % 2 === 0;
          r.innerHTML = shown.replace(/^\$/, '<span style="color:#ef7d3e">$</span>') + (caret ? '<span style="background:#eef1f4">&nbsp;</span>' : "");
        } else {
          shown = t >= t0 ? txt : "";
          if (kind === "kv" && shown) {
            const m = shown.match(/^(\s+[A-Za-z ]+:)(\s+)(.*)$/);
            if (m) shown = `<span style="color:#6f8499">${m[1]}</span>${m[2]}<span class="v">${m[3]}</span>`;
          }
          if (r.__h !== shown) { r.__h = shown; r.innerHTML = shown; }
        }
      });
      // measure the Dashboard URL span once it exists (offsets ignore transforms)
      if (!s.dash && t >= 10.375) {
        const v = s.rows[7].querySelector(".v");
        if (v) s.dash = { x: v.offsetLeft, w: v.offsetWidth };
      }
      const rowY = (i) => 80 + i * FS * LH + (FS * LH) / 2;
      // which row the camera follows
      const active = LINES.reduce((acc, l, i) => (t >= l[0] ? i : acc), 0);
      // --- camera over the terminal. Keep the text's left edge (plane x=110)
      // just right of the statement band once statements are on screen.
      const zBase = L.portrait ? 0.78 : 1.2;
      const follow = track(t, LINES.map((l, i) => [l[0], rowY(Math.min(i, 9))]), 170, 26);
      const dashY = rowY(7);
      const leftAt = (screenX, z, pxC) => 110 + (pxC - screenX) / z; // cx so plane x=110 lands at screenX
      const zA = zBase, zB = zBase * 0.8;
      const pxA = L.W * 0.5, pxB = L.portrait ? L.W * 0.5 : L.W * 0.62;
      const pyB = L.portrait ? L.H * 0.66 : L.H * 0.52;
      const keys = [
        [7.95, { cx: leftAt(L.m, zA * 1.6, pxA), cy: rowY(0), z: zA * 1.6, rx: 14, ry: -18, rz: -2, px: pxA, py: L.H * 0.5 }],
        [8.2, { rx: 8, ry: -10, rz: -1 }],
        [9.0, { cx: leftAt(L.m, zA, pxA), z: zA }],
        [10.0, { cx: leftAt(L.portrait ? L.m * 0.5 : L.W * 0.4, zB, pxB), px: pxB, py: pyB, ry: -12, z: zB }],
        [12.0, { cx: s.dash ? s.dash.x + s.dash.w * 0.5 : 1300, px: L.W * 0.5, py: L.H * 0.5, z: zBase * 1.35, rx: 3, ry: -6, rz: 0 }],
        [12.55, { z: zBase * 2.8, rx: 0, ry: 0 }],
      ];
      const cam = camTrack(t, keys);
      cam.cy = t < 12.0 ? follow : track(t, [[0, follow], [12.0, dashY]], 210, 30);
      // ride the caret while the install command types (8.0–9.0)
      if (t < 9.3) {
        const typed = s.rows[0].textContent.length;
        const caretX = 110 + typed * s.glyph;
        const ride = leftAt(L.m, zA * 1.6, pxA) + Math.max(0, caretX - (110 + (pxA + L.W * 0.18 - L.m) / (zA * 1.6)));
        const w = 1 - clamp((t - 9.0) / 0.3);
        cam.cx = cam.cx * (1 - w) + track(t, [[0, ride]], 170, 26) * 0 + ride * w;
      }
      const m = shotMatrix({ ...cam, persp: 1700 });
      S(s.term, { transform: css(m), display: t < 12.95 ? "" : "none" });
      // highlight ring on the Dashboard URL
      const rp = snappy(t - 12.1);
      show(s.ring, t >= 12.1);
      const d = s.dash || { x: 1100, w: 800 };
      S(s.ring, { left: px(d.x - 22), top: px(dashY - FS * 0.78), width: px((d.w + 44) * clamp(rp)), height: px(FS * 1.56) });
            // cursor to the URL and click on 12.5
      const target = project(m, d.x + d.w * 0.55, dashY + FS * 0.2);
      drawCursor(s.cur, t, [[11.6, L.W * 0.9, L.H * 1.1], [12.2, target[0] + 20, target[1] + 30], [12.5, target[0], target[1]]], [12.5], L.u * 1.2, t >= 11.6 && t < 12.95);
      // --- statements
      const sx = L.m, sy = L.portrait ? L.H * 0.07 : L.H * 0.23;
      const ssize = (L.portrait ? 170 : 250) * L.u;
      drawStatement(s.st1, t, 10.0, 12.0, { x: sx, y: sy, size: ssize, color: "#eef1f4", lh: 0.86 });
      const s2y = L.portrait ? sy + ssize * 1.0 : sy + ssize * 1.85;
      drawStatement(s.st2, t, 11.0, 12.0, { x: sx, y: s2y, size: (L.portrait ? 78 : 76) * L.u, color: "#93a4b6" });
      // --- the real Overview swings in 12.5 → 13.0, glides to Collections by 14.0
      const on = t >= 12.5;
      show(s.ov.root, on);
      if (on) {
        const ov = s.ov;
        const coll = ov.union(ov.box("Collections", { kind: "text", n: 2 }), ov.box("posts", { kind: "text", n: 1 }), ov.box("Read access"));
        const head = ov.union(ov.box("Overview", { kind: "text", n: 2 }), ov.box("Requests per hour"));
        const f1 = frame(head, L.full, { fill: 0.95 });
        const f2 = frame(coll, L.full, { fill: 0.9, minZoom: L.portrait ? 0.6 : 0.8 });
        const ck = [
          [12.5, { ...f1, z: f1.z * 0.55, rx: 38, ry: 24, rz: -6, lift: -900 }],
          [12.62, { ...f1, rx: 6, ry: -4, rz: 0, lift: 0 }],
          [13.25, { ...f2, rx: 10, ry: -10 }],
        ];
        const c = camTrack(t, ck);
        ov.set(shotMatrix({ ...c, persp: 2000 }));
      }
    },
  },
];
