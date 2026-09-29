// Chapter 1 — Hook (0–8 s). docs/shotlist.md §1.
//  0.0–2.0  "Auth." "Files." "Realtime." "Email." — one mega word per beat,
//           each shoving the stack up (one track() on its y).
//  2.0–4.0  hard cut to fog: "Stop rebuilding / your backend."
//  4.0–6.0  the crate drops and lands on the 4.0 hit; wordmark on 5.0.
//  6.0–8.0  site hero line on 6.0; camera pushes into the crate's top face,
//           which opens into the terminal's darkness by 8.0.

import { el, S, place, show, px, maskLine, fitWidth, heavy, snappy, soft, prog, lerp, easeIn, mixColor, crateSVG, statement, drawStatement } from "../lib/kit.js";
import { track, clamp, spring } from "../lib/motion.js";

const WORDS = ["Auth.", "Files.", "Realtime.", "Email."];
const WORD_T = [0, 0.5, 1.0, 1.5];

export default [
  // ------------------------------------------------------------ 1.1 words
  {
    id: "hook-words",
    from: 0,
    to: 2,
    build(root, L) {
      S(root, { background: "var(--navy)" });
      const stack = el("div", "abs", root);
      // size the mega words so the longest ("Realtime.") spans the frame width
      const probe = el("div", "abs disp nowrap", root, "Realtime.");
      const size = fitWidth(probe, L.W - 2 * L.m * 0.9, 300);
      probe.remove();
      const lh = size * 0.86;
      const lines = WORDS.map((w, i) => {
        const ln = maskLine(stack, "disp", w);
        S(ln.wrap, { fontSize: px(size), top: px(i * lh), paddingBottom: px(size * 0.1), left: "0px" });
        return ln;
      });
      return { stack, lines, size, lh };
    },
    draw(t, s, L) {
      // the stack's y: word i's baseline sits at the frame's optical centre
      const base = L.H * (L.portrait ? 0.56 : 0.6) - s.lh * 0.8;
      const y = track(t, WORD_T.map((tt, i) => [tt, base - i * s.lh]), 260, 32);
      place(s.stack, L.m * 0.9, y);
      s.lines.forEach((ln, i) => {
        const p = heavy(t - WORD_T[i] + 0.02);
        S(ln.inner, { transform: `translate3d(0,${((1 - p) * 125).toFixed(2)}%,0)` });
        // current word bright, older words recede into the navy
        const age = clamp((t - WORD_T[i]) / 0.5) * (i < 3 ? 1 : 0);
        const older = t >= (WORD_T[i + 1] ?? 99) ? 1 : 0;
        S(ln.wrap, { color: mixColor("#eef1f4", "#2a4058", older * clamp((t - WORD_T[i + 1]) / 0.25)) });
        void age;
      });
    },
  },

  // ------------------------------------------------------------ 1.2 stop rebuilding
  {
    id: "hook-stop",
    from: 2,
    to: 4,
    build(root, L) {
      S(root, { background: "var(--fog)" });
      const lines = L.portrait ? ["Stop", "rebuilding", "your", "backend."] : ["Stop rebuilding", "your backend."];
      const box = el("div", "abs", root);
      const probe = el("div", "abs disp nowrap", root, L.portrait ? "rebuilding" : "Stop rebuilding");
      const size = fitWidth(probe, L.W - 2 * L.m, 240);
      probe.remove();
      const ls = lines.map((w, i) => {
        const ln = maskLine(box, "disp", w);
        S(ln.wrap, { fontSize: px(size), top: px(i * size * 0.88), color: "var(--harbor)", paddingBottom: px(size * 0.1) });
        return ln;
      });
      return { box, ls, size };
    },
    draw(t, s, L) {
      const n = s.ls.length;
      const blockH = n * s.size * 0.88;
      // falling-crate shadow shoves the type down on 3.5
      const shove = easeIn(prog(t, 3.5, 4.0)) * L.H * 1.1;
      place(s.box, L.m, (L.H - blockH) / 2 - s.size * 0.06 + shove);
      const starts = n === 2 ? [2.0, 2.5] : [2.0, 2.25, 2.5, 2.75];
      s.ls.forEach((ln, i) => {
        const p = heavy(t - starts[i] + 0.02);
        S(ln.inner, { transform: `translate3d(0,${((1 - p) * 125).toFixed(2)}%,0)` });
      });
    },
  },

  // ------------------------------------------------------------ 1.3–1.4 crate
  {
    id: "hook-crate",
    from: 3.5,
    to: 8,
    z: 2,
    build(root, L) {
      const bg = el("div", "abs", root);
      S(bg, { inset: "0", width: "100%", height: "100%" });
      // world group that the camera scales (push into the top face)
      const cam = el("div", "abs", root);
      S(cam, { transformOrigin: "0 0" });
      const U = (L.portrait ? 360 : 410) * L.u;
      const g = crateSVG(U);
      const P = g.P;
      // crate footprint in its own coords: x from -U..U, y from -V..(2H)
      const svgW = U * 2 + 40, top = P([0, 0, 1])[1], bottom = P([1, 1, 0])[1];
      const svgH = bottom - top + 40;
      const svg = el("div", "abs", cam, `<svg width="${svgW}" height="${svgH}" viewBox="${-U - 20} ${top - 20} ${svgW} ${svgH}" style="overflow:visible">
        <path d="${g.left}" fill="#cf703d" stroke="#eef1f4" stroke-width="${(4 * L.u).toFixed(1)}" stroke-linejoin="round"/>
        <path d="${g.right}" fill="#ac623c" stroke="#eef1f4" stroke-width="${(4 * L.u).toFixed(1)}" stroke-linejoin="round"/>
        <path d="${g.top}" fill="#ef7d3e" stroke="#eef1f4" stroke-width="${(4 * L.u).toFixed(1)}" stroke-linejoin="round"/>
        ${g.br.map((d, i) => `<path class="br" data-i="${i}" d="${d}" fill="none" stroke="#eef1f4" stroke-width="${(3 * L.u).toFixed(1)}" stroke-linecap="round" opacity="0.55" pathLength="1" stroke-dasharray="1 1" stroke-dashoffset="1"/>`).join("")}
      </svg>`);
      const shadow = el("div", "abs", cam);
      const brs = [...svg.querySelectorAll(".br")];
      // crate placement: right half in 16:9, lower-middle in 9:16
      const cx = L.portrait ? L.W * 0.5 : L.W * 0.735;
      const cy = L.portrait ? L.H * 0.64 : L.H * 0.5; // screen y of the crate's centre
      const topFace = { x: 0, y: P([0.5, 0.5, 1])[1] }; // top-face centre in crate coords
      const word = maskLine(root, "disp", "Cratebase");
      const probe = el("div", "abs disp nowrap", root, "Cratebase");
      const wsize = L.portrait ? fitWidth(probe, L.W - 2 * L.m, 200) : fitWidth(probe, L.W * 0.4, 200);
      probe.remove();
      S(word.wrap, { fontSize: px(wsize), paddingBottom: px(wsize * 0.12) });
      const tag = statement(root, L.portrait ? ["A backend", "in one crate."] : ["A backend in one crate."]);
      const hole = el("div", "abs", root);
      S(hole, { background: "#070d14" });
      return { bg, cam, svg, svgW, svgH, U, top, cx, cy, topFace, brs, word, wsize, tag, hole, shadow, P };
    },
    draw(t, s, L) {
      const pre = t < 4;
      S(s.bg, { background: pre ? "transparent" : "var(--navy)" });
      // fall: accelerating (gravity) from above the frame, landing exactly at 4.0
      const fall = pre ? easeIn(prog(t, 3.5, 4.0)) : 1;
      const dropY = (1 - fall) * -(L.H * 1.2);
      // tiny squash on landing (heavy spring, settles, no bounce)
      const land = t >= 4 ? spring(t - 4, 260, 26) : 0;
      const squash = t >= 4 ? 1 - 0.05 * Math.exp(-(t - 4) * 10) : 1;
      void land;
      // camera push into the top face 7.0→7.7 (exponential zoom)
      const pz = prog(t, 7.0, 7.75);
      const dolly = 1 + 0.07 * prog(t, 4.0, 7.2);
      const zoom = Math.exp(easeIn(pz) * Math.log(16)) * dolly;
      // crate svg positioned so its box centre is at (cx, cy)
      const sx = s.cx - s.svgW / 2, sy = s.cy - s.svgH / 2 + dropY;
      S(s.svg, { transform: `translate3d(${px(sx)},${px(sy)},0) scale(1,${squash.toFixed(4)})`, transformOrigin: "50% 100%" });
      // push focus: the top-face centre, mapped through the svg viewBox
      const fx = sx + s.U + 20, fy = s.cy - s.svgH / 2 + s.P([0.5, 0.5, 1])[1] - s.top + 20;
      // the push also travels the face centre to frame centre, where the hole opens
      const e = easeIn(pz) ** 0.6;
      const tx = fx + (L.W / 2 - fx) * e, ty = fy + (L.H / 2 - fy) * e;
      S(s.cam, { transform: `translate3d(${px(tx)},${px(ty)},0) scale(${zoom.toFixed(4)}) translate3d(${px(-fx)},${px(-fy)},0)` });
      // braces draw on 4.5 (one per 16th)
      s.brs.forEach((b, i) => {
        const p = clamp(soft(t - 4.5 - i * 0.125));
        b.setAttribute("stroke-dashoffset", (1 - p).toFixed(3));
      });
      // wordmark rises on 5.0, statement on 6.0; both leave with the push
      const out = easeIn(pz);
      const wx = L.portrait ? L.m : L.m;
      const wy = L.portrait ? L.H * 0.12 : L.H * 0.5 - s.wsize * 0.78;
      show(s.word.wrap, t >= 4.9);
      const drift = -38 * L.u * prog(t, 4.9, 7.2);
      place(s.word.wrap, wx - out * L.W * 0.6 + drift, wy);
      S(s.word.inner, { transform: `translate3d(0,${((1 - heavy(t - 5.0)) * 125).toFixed(2)}%,0)` });
      const tsize = (L.portrait ? 92 : 104) * L.u;
      drawStatement(s.tag, t, 6.0, 999, {
        x: wx - out * L.W * 0.6 + drift * 1.6,
        y: wy + s.wsize * 0.98,
        size: tsize,
        color: "#93a4b6",
      });
      // the top face opens into darkness (the terminal) 7.55→8.0
      const hp = soft(t - 7.55);
      show(s.hole, t >= 7.55);
      if (t >= 7.55) {
        const w = L.W * hp * 1.02, h = L.H * hp * 1.02;
        S(s.hole, { left: px((L.W - w) / 2), top: px((L.H - h) / 2), width: px(w), height: px(h), borderRadius: px((1 - hp) * 40) });
      }
    },
  },
];
