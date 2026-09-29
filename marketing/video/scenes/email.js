// Chapter 5 — Email (30–36 s). docs/shotlist.md §5.
// REAL: the welcome template in the dashboard's HTML editor (scrolled to
// `Welcome, {{user.name}}` by capture.mjs), and the dev inbox with the real
// mail sent by capture/seed.sh through POST /api/mails/send — the same call
// `cb.mails.send({ template, to, data })` makes (sdk/js/client/src/cratebase-only.ts).

import { el, S, place, show, px, heavy, snappy, soft, prog, statement, drawStatement, callout, drawCallout, codeCard, drawCode, easeIn } from "../lib/kit.js";
import { shotMatrix, camTrack, project, frame } from "../lib/cam.js";
import { UIPlane, loadShot } from "../lib/ui.js";
import { clamp } from "../lib/motion.js";
import { addCue } from "../lib/timeline.js";

const SEND = `await cb.mails.send({
  template: "welcome",
  to: "amara@cratebase.dev",
  data: { user: { name: "Amara" } },
});`;
const SEND_T0 = 32.1, SEND_T1 = 33.25;

addCue(30.0, "whoosh", { dur: 0.45, gain: 0.6 });
addCue(30.75, "pop", { note: 74 });
addCue(31.0, "whoosh", { dur: 0.3, gain: 0.35 });
addCue(32.0, "whoosh", { dur: 0.35, gain: 0.5 });
addCue(SEND_T0, "keys", { n: 20, dur: SEND_T1 - SEND_T0 });
addCue(33.5, "send", {});
addCue(34.0, "pop", { note: 81 });
addCue(35.0, "confirm", {});

export default [
  {
    id: "email",
    from: 30,
    to: 36,
    z: 3,
    async build(root, L) {
      S(root, { background: "#050507" });
      const ed = new UIPlane(root, "hd-email-editor", await loadShot("hd-email-editor"));
      const inbox = new UIPlane(root, "hd-mail-inbox", await loadShot("hd-mail-inbox"));
      // `{{user.name}}` inside the measured line-1 run "Welcome, {{user.name}}</"
      const run = ed.box("Welcome, {{user.name}}");
      const adv = run.w / "Welcome, {{user.name}}</".length;
      const varBox = { x: run.x + adv * 9, y: run.y, w: adv * 13, h: run.h };
      const ring = ed.rectEl({ x: 0, y: 0, w: 0, h: 0 }, { border: "8px solid #ef7d3e", borderRadius: "14px" });
      // the chip that lifts off the plane toward the camera (crisp DOM text)
      const chip = el("div", "abs nowrap", root, "{{user.name}}");
      S(chip, { fontFamily: "JBMono, monospace", fontWeight: 600, color: "#0a1622", background: "#ef7d3e", borderRadius: "0.3em", padding: "0.12em 0.4em 0.18em", boxShadow: "0 30px 70px rgba(0,0,0,.55)", transformOrigin: "0 0" });
      // inbox: the "Welcome to Acme" row arrives on 34.0 (row mask) — plane px
      const row = inbox.box("Welcome to Acme", { n: 0 });
      const rowMask = inbox.rectEl({ x: row.x - 60, y: row.y - 40, w: 1000, h: 170 }, { background: "#18181b" });
      // "Welcome, Amara" inside the rendered mail (iframe; measured off the capture)
      const hello = inbox.rect(690, 481, 176, 25);
      const code = codeCard(root, SEND);
      const st = {
        tpl: statement(root, L.portrait ? ["Templates with", "variables."] : ["Templates with variables."]),
        send: statement(root, L.portrait ? ["Send it from", "your frontend."] : ["Send it from your frontend."]),
        inbox: statement(root, L.portrait ? ["A dev inbox,", "built in."] : ["A dev inbox, built in."]),
      };
      const co = callout(root, "{{user.name}} → Amara", { mono: true, dark: false });
      return { ed, inbox, varBox, ring, chip, row, rowMask, hello, code, st, co };
    },
    draw(t, s, L) {
      const P = L.portrait;
      const { ed, inbox } = s;
      show(s.co.g, false);
      // ------------------------------------------------ editor 30–34
      const onEd = t < 34.1;
      show(ed.root, onEd);
      let chipAnchor = null;
      if (onEd) {
        const edArea = ed.union(ed.box("HTML", { kind: "text", n: 1 }), ed.box("Insert variable", { kind: "button" }), ed.rect(260, 330, 580, 200));
        const line = { x: s.varBox.x - 330, y: s.varBox.y - 60, w: s.varBox.w + 560, h: s.varBox.h + 120 };
        const fA = frame(edArea, L.below, { fill: 0.95 });
        const fB = frame(line, L.below, { fill: 0.95 });
        const keys = [
          [29.9, { ...fA, z: fA.z * 0.55, rx: -24, ry: 28, rz: -3, lift: -500 }],
          [30.0, { ...fA, rx: 6, ry: 8, rz: 0, lift: 0 }],
          [30.4, { ...fB, rx: 3, ry: 4 }],
          [32.0, { z: fB.z * 0.5, rx: 22, ry: -26, py: L.below.y + L.below.h * 0.45 }],
        ];
        const cam = camTrack(t, keys);
        const m = shotMatrix({ ...cam, persp: 2200 });
        ed.set(m);
        ed.spot({ x: line.x - 200, y: line.y, w: line.w + 400, h: line.h }, clamp((t - 30.05) / 0.3) * 0.92 * (1 - clamp((t - 32) / 0.3)));
        // ring locks on 30.75
        const rp = snappy(t - 30.75);
        show(s.ring, t >= 30.75 && t < 32.0);
        const g = 26 * (1 - rp) + 10;
        S(s.ring, { left: px(s.varBox.x - g), top: px(s.varBox.y - g), width: px(s.varBox.w + 2 * g), height: px(s.varBox.h + 2 * g), opacity: String(clamp(rp * 2)) });
        const a = project(m, s.varBox.x, s.varBox.y);
        const b = project(m, s.varBox.x + s.varBox.w, s.varBox.y + s.varBox.h);
        chipAnchor = { x: a[0], y: a[1], w: b[0] - a[0], h: b[1] - a[1] };
      }
      // chip lifts off on 31.0 and hangs in front of the code card until 33.5
      const chipOn = t >= 31.0 && t < 33.6 && chipAnchor;
      show(s.chip, !!chipOn);
      if (chipOn) {
        const p = heavy(t - 31.0);
        const baseSize = chipAnchor.h * 0.78;
        const targetSize = (P ? 70 : 96) * L.u;
        const size = baseSize + (targetSize - baseSize) * p;
        const tx = P ? L.m * 0.6 : L.W * 0.16, ty = P ? L.H * 0.3 : L.H * 0.235;
        const fling = easeIn(prog(t, 33.4, 33.6));
        S(s.chip, { fontSize: px(size) });
        place(s.chip, chipAnchor.x + (tx - chipAnchor.x) * p + fling * L.W, chipAnchor.y + (ty - chipAnchor.y) * p, ` rotate(${(p * -4).toFixed(2)}deg)`);
      }
      // ------------------------------------------------ send code card 32–34
      const onCode = t >= 32.0 && t < 33.75;
      show(s.code.card, onCode);
      if (onCode) {
        const p = heavy(t - 32.0);
        const fling = easeIn(prog(t, 33.4, 33.7));
        const size = (P ? 38 : 56) * L.u;
        S(s.code.card, { fontSize: px(size) });
        place(s.code.card, (P ? L.m * 0.6 : L.W * 0.16) + fling * L.W * 1.2, (P ? L.H * 0.4 : L.H * 0.36) + (1 - p) * L.H * 0.8, ` rotate(${((1 - p) * 5 + fling * 8).toFixed(2)}deg)`);
        drawCode(s.code, SEND.length * (Math.floor(clamp((t - SEND_T0) / (SEND_T1 - SEND_T0)) * 20) / 20));
      }
      // ------------------------------------------------ inbox 34–36
      const onIn = t >= 33.55;
      show(inbox.root, onIn);
      if (onIn) {
        const list = inbox.union(inbox.box("Subject", { n: 0 }), inbox.box("amara@cratebase.dev", { n: 0 }), s.hello);
        const paneHead = inbox.box("Welcome to Acme", { n: 1 });
        const hi = { x: paneHead.x - 20, y: paneHead.y - 40, w: 1236 * 4 - paneHead.x, h: s.hello.y + s.hello.h + 160 - paneHead.y };
        const fA = frame(list, L.below, { fill: 0.95 });
        const fB = frame(hi, L.below, { fill: 0.95 });
        const keys = [
          [33.55, { ...fA, cx: fA.cx - 2600, z: fA.z * 0.8, rx: 8, ry: 34, lift: -300 }],
          [33.75, { ...fA, rx: 6, ry: -8, lift: 0 }],
          [34.5, { ...fB, rx: 3, ry: -4 }],
        ];
        const cam = camTrack(t, keys);
        const m = shotMatrix({ ...cam, persp: 2200 });
        inbox.set(m);
        const focus = t < 34.4 ? list : hi;
        inbox.spot({ x: focus.x - 60, y: focus.y - 40, w: focus.w + 120, h: focus.h + 80 }, 0.86 * clamp((t - 33.6) / 0.25));
        const x0 = s.row.x - 60, x1 = inbox.box("Sent", { n: 0 }).x + 420, rp = heavy(t - 34.0);
        S(s.rowMask, { left: px(x0 + (x1 - x0) * rp), width: px((x1 - x0) * (1 - rp)) });
        const am = project(m, s.hello.x + s.hello.w * 0.8, s.hello.y + s.hello.h);
        drawCallout(s.co, t, 35.0, 36.0, am, P ? [-300 * L.u, -150 * L.u] : [-80 * L.u, -170 * L.u], (P ? 50 : 64) * L.u);
      }
      // ------------------------------------------------ statements
      const o = { x: L.top.x, y: L.top.y, size: (P ? 92 : 112) * L.u, color: "#eef1f4" };
      drawStatement(s.st.tpl, t, 30.0, 32.0, o);
      drawStatement(s.st.send, t, 32.0, 34.0, o);
      drawStatement(s.st.inbox, t, 34.0, 36.0, o);
    },
  },
];
