// Chapter 3 — Data (14–22 s). docs/shotlist.md §3. All UI pixels are the
// real v0.4.0 dashboard (capture/shots/hd-*.png, DPR 4). Motion on top of
// the captures is limited to camera, spotlight, and reveal masks painted
// in the page's own measured colours (#09090b page, #1b1b1e field input,
// #111113 rule input) — nothing of the product is redrawn.

import { el, S, place, show, px, heavy, snappy, soft, prog, statement, drawStatement, callout, drawCallout, codeCard, drawCode, cursor, drawCursor } from "../lib/kit.js";
import { shotMatrix, camTrack, project, frame } from "../lib/cam.js";
import { UIPlane, loadShot } from "../lib/ui.js";
import { clamp, track } from "../lib/motion.js";
import { addCue } from "../lib/timeline.js";

const RULE = "owner = @request.auth.id";
const RULE_T0 = 16.5, RULE_T1 = 17.25; // typed in 16th-note bursts

const CODE = `const places = await cb
  .collection("places")
  .list({ sort: "-created" });
// cratebase typegen
type PlacesRecord = {
  id: string;
  name: string;
  description?: string;
  location?: { lon: number; lat: number };
  owner?: string;
};`;
const CODE_T0 = 20.0, CODE_T1 = 21.25;

[14.0, 14.5, 15.0, 15.5].forEach((t, i) => addCue(t, "pop", { note: 69 + [0, 3, 5, 7][i] }));
addCue(14.0, "whoosh", { dur: 0.45, gain: 0.6 });
addCue(16.0, "whoosh", { dur: 0.4, gain: 0.5 });
addCue(RULE_T0, "keys", { n: 14, dur: RULE_T1 - RULE_T0 });
addCue(17.25, "confirm", {});
addCue(18.0, "whoosh", { dur: 0.5, gain: 0.7 });
[18.1, 18.3, 18.5, 18.7].forEach((t, i) => addCue(t, "tick", { note: 81 + i * 2 }));
addCue(20.0, "whoosh", { dur: 0.35, gain: 0.5 });
addCue(CODE_T0 + 0.1, "keys", { n: 16, dur: CODE_T1 - CODE_T0 });

export default [
  {
    id: "data",
    from: 14,
    to: 22,
    z: 3,
    async build(root, L) {
      S(root, { background: "#050507" });
      const sch = new UIPlane(root, "hd-places-schema", await loadShot("hd-places-schema"));
      const rec = new UIPlane(root, "hd-places-records", await loadShot("hd-places-records"));
      // field-row reveal masks (one per row, page colour, wiping left→right)
      const rowsR = ["name", "description", "location", "owner"].map((n) => {
        const inp = sch.box(n, { kind: "input" });
        return sch.rectEl({ x: inp.x - 200, y: inp.y - 30, w: sch.w - inp.x + 100, h: inp.h + 60 }, { background: "#09090b" });
      });
      // rule text mask: covers the real Update rule text, shrinks per 16th
      const upd = sch.box(RULE, { kind: "input", n: 0 });
      const ruleMask = sch.rectEl({ x: upd.x + 40, y: upd.y + 8, w: 1400, h: upd.h - 16 }, { background: "#111113" });
      const caret = sch.rectEl({ x: 0, y: upd.y + 26, w: 8, h: upd.h - 52 }, { background: "#ef7d3e" });
      // record row masks
      const recRows = ["Kopi Kenangan Senopati", "Pasar Santa Vinyl Corner", "Blok M Co-working Loft", "Warung Tekko"].map((n) => {
        const b = rec.box(n);
        return rec.rectEl({ x: 600, y: b.y - 40, w: rec.w - 600, h: b.h + 64 }, { background: "#09090b" });
      });
      const rings = [0, 1].map(() => sch.rectEl({ x: 0, y: 0, w: 0, h: 0 }, { border: "10px solid #ef7d3e", borderRadius: "22px" }));
      const st = {
        fields: statement(root, ["Model your data."]),
        rules: statement(root, L.portrait ? ["Rules are filter", "expressions."] : ["Rules are filter expressions."]),
        api: statement(root, L.portrait ? ["A REST API for", "every collection."] : ["A REST API for every collection."]),
        types: statement(root, L.portrait ? ["Typed SDK,", "generated types."] : ["Typed SDK, generated types."]),
      };
      const co = {
        owner: callout(root, "Only the owner can edit."),
        geo: callout(root, "Geo point", { mono: false }),
      };
      const code = codeCard(root, CODE);
      return { sch, rec, rowsR, ruleMask, caret, recRows, rings, st, co, code, upd };
    },
    draw(t, s, L) {
      const { sch, rec } = s;
      const P = L.portrait;
      const below = L.below;
      show(s.co.owner.g, false);
      show(s.co.geo.g, false);
      // ---------------------------------------------------- schema plane 14–18
      const onSch = t < 18.15;
      show(sch.root, onSch);
      if (onSch) {
        const fields = sch.union(sch.box("name", { kind: "input" }), sch.box("owner", { kind: "input" }), sch.box("Relation", { kind: "button" }), sch.box("→ users"));
        const rule = sch.union(sch.box("Update"), sch.box(RULE, { kind: "input", n: 0 }));
        const ruleTight = { x: rule.x, y: rule.y, w: 380 * 4, h: rule.h };
        if (P) { ruleTight.w = 250 * 4; }
        const fF = frame(fields, below, { fill: 0.94 });
        const fR = frame(ruleTight, { ...below, h: below.h * 0.7 }, { fill: 0.9 });
        // portrait: rack across the row instead of shrinking it — names, then types
        const namesCol = sch.union(sch.box("name", { kind: "input" }), sch.box("owner", { kind: "input" }));
        const typesCol = sch.union(sch.box("Text", { kind: "button" }), sch.box("Relation", { kind: "button" }), sch.box("→ users"));
        const fN = frame({ ...namesCol, w: namesCol.w * 0.8 }, below, { fill: 0.96 });
        const fT = frame({ x: typesCol.x - 300, y: typesCol.y, w: typesCol.w + 300, h: typesCol.h }, below, { fill: 0.94 });
        const keys = P ? [
          [13.9, { ...fN, z: fN.z * 0.45, rx: 30, ry: -26, rz: 4, lift: -400 }],
          [14.0, { ...fN, rx: 6, ry: -10, rz: 0, lift: 0 }],
          [14.9, { ...fT, rx: 6, ry: 10 }],
          [16.0, { ...fR, rx: 6, ry: 6 }],
          [17.5, { z: fR.z * 1.06, rx: 2, ry: 2 }],
          [17.95, { cx: fR.cx + 2400, z: fR.z * 0.8, ry: -30 }],
        ] : [
          [13.9, { ...fF, z: fF.z * 0.45, rx: 30, ry: -26, rz: 4, lift: -400 }],
          [14.0, { ...fF, rx: 8, ry: -8, rz: 0, lift: 0 }],
          [15.9, { rx: 4, ry: -4, z: fF.z * 1.04 }],
          [16.0, { ...fR, rx: 6, ry: 6 }],
          [17.5, { z: fR.z * 1.06, rx: 2, ry: 2 }],
          [17.95, { cx: fR.cx + 2400, z: fR.z * 0.8, ry: -30 }],
        ];
        const cam = camTrack(t, keys);
        const m = shotMatrix({ ...cam, persp: 2200 });
        sch.set(m);
        // spotlight follows the focus region
        const focus = t < 16 ? fields : rule;
        const pad = 60;
        sch.spot({ x: focus.x - pad, y: focus.y - pad, w: focus.w + 2 * pad, h: focus.h + 2 * pad }, 0.9 * clamp((t - 13.95) / 0.3));
        // rows unmask one per beat
        s.rowsR.forEach((r, i) => {
          const p = heavy(t - 14.0 - i * 0.5);
          S(r, { transform: `translate3d(${(p * 4200).toFixed(1)}px,0,0)` });
        });
        // rings on Geo point (15.0) and Relation → users (15.5)
        const ringFor = (i, b, t0) => {
          const on = t >= t0 && t < 16.0;
          show(s.rings[i], on);
          if (!on) return;
          const p = snappy(t - t0);
          const g = 22 * (1 - p) + 14;
          S(s.rings[i], { left: px(b.x - g), top: px(b.y - g), width: px(b.w + 2 * g), height: px(b.h + 2 * g), opacity: String(clamp(p * 2)) });
        };
        ringFor(0, sch.box("Geo point", { kind: "button" }), 15.0);
        ringFor(1, sch.union(sch.box("Relation", { kind: "button" }), sch.box("→ users")), 15.5);
        // rule typing
        const ruleBox = sch.box(RULE, { kind: "input", n: 0 });
        const glyphW = (ruleBox.w > 0 ? 8.43 : 8.43) * 4; // JetBrains Mono 14px advance ≈ 8.43 CSS px
        const q = Math.floor(clamp((t - RULE_T0) / (RULE_T1 - RULE_T0)) * 12) / 12;
        const n = Math.round(RULE.length * q);
        const x0 = ruleBox.x + 13 * 4 + n * glyphW;
        S(s.ruleMask, { left: px(x0), top: px(ruleBox.y + 10), display: t < RULE_T1 ? "" : "none" });
        S(s.caret, { left: px(x0 + 2), top: px(ruleBox.y + 22), display: t >= 16.2 && t < RULE_T1 + 0.5 && Math.floor(t * 4) % 2 === 0 ? "" : "none" });
        // callout on the rule at 17.25
        const a = project(m, ruleBox.x + 13 * 4 + RULE.length * glyphW * 0.72, ruleBox.y + ruleBox.h);
        drawCallout(s.co.owner, t, 17.25, 18.0, a, P ? [-160 * L.u, 170 * L.u] : [140 * L.u, 130 * L.u], (P ? 58 : 68) * L.u);
      }
      // ---------------------------------------------------- records plane 18–22
      const onRec = t >= 17.85;
      show(rec.root, onRec);
      if (onRec) {
        const cols = rec.union(rec.box("name", { kind: "text" }), rec.box("Warung Tekko"), rec.box("-6.2297"));
        const cols2 = rec.union(rec.box("location", { kind: "text", n: 0 }), rec.box("Amara Chen", { n: 1 }), rec.box("-6.2297"));
        const colsP = rec.union(rec.box("name", { kind: "text" }), rec.box("Pasar Santa Vinyl Corner"), rec.box("Warung Tekko"));
        const fA = frame(P ? colsP : cols, below, { fill: 0.94 });
        const cols2P = rec.union(rec.box("location", { kind: "text", n: 0 }), rec.box("-6.2297"), rec.box("-6.2407"));
        const fB = frame(P ? { ...cols2P, w: cols2P.w + 360 } : cols2, below, { fill: 0.94 });
        const keys = [
          [17.85, { ...fA, cx: fA.cx - 2600, z: fA.z * 0.8, rx: 6, ry: 30, lift: -200 }],
          [18.0, { ...fA, rx: 6, ry: -6, lift: 0 }],
          [19.0, { ...fB, rx: 4, ry: -8 }],
          [20.0, { z: fB.z * 0.78, rx: 14, ry: -14, py: below.y + below.h * 0.62 }],
        ];
        const cam = camTrack(t, keys);
        const m = shotMatrix({ ...cam, persp: 2200 });
        rec.set(m);
        const tbl = rec.union(rec.box("name", { kind: "text" }), rec.box("Amara Chen", { n: 1 }), rec.box("Warung Tekko"));
        rec.spot({ x: tbl.x - 700, y: tbl.y - 30, w: tbl.w + 900, h: tbl.h + 60 }, 0.84 * clamp((t - 17.9) / 0.25));
        s.recRows.forEach((r, i) => S(r, { transform: `translate3d(${(heavy(t - 18.1 - i * 0.2) * 4800).toFixed(1)}px,0,0)` }));
      }
      // ---------------------------------------------------- code card 20–22
      const onCode = t >= CODE_T0 - 0.05;
      show(s.code.card, onCode);
      if (onCode) {
        const p = heavy(t - CODE_T0);
        const size = (P ? 36 : 50) * L.u;
        S(s.code.card, { fontSize: px(size) });
        const x = P ? L.m * 0.6 : L.W * 0.26;
        const y = P ? L.H * 0.3 : L.H * 0.215;
        place(s.code.card, x, y + (1 - p) * L.H * 0.7, ` rotate(${((1 - p) * -6).toFixed(2)}deg)`);
        const q = Math.floor(clamp((t - CODE_T0 - 0.1) / (CODE_T1 - CODE_T0)) * 20) / 20;
        drawCode(s.code, CODE.length * q, true);
      }
      // ---------------------------------------------------- statements (top band)
      const sz = (P ? 92 : 112) * L.u;
      const o = { x: L.top.x, y: L.top.y, size: sz, color: "#eef1f4" };
      drawStatement(s.st.fields, t, 14.0, 16.0, o);
      drawStatement(s.st.rules, t, 16.0, 18.0, o);
      drawStatement(s.st.api, t, 18.0, 20.0, o);
      drawStatement(s.st.types, t, 20.0, 22.0, o);
    },
  },
];
