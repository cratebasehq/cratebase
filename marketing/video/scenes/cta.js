// Chapter 8 — CTA (56–64 s). docs/shotlist.md §8. The crate lands again on
// the 56.0 hit (rhyme with 4.0), then the lockup builds on the beat and the
// camera pushes in slowly until the last frame, which is the poster.
// Links and claims: site/astro.config.mjs (cratebase.dev), site nav
// (github.com/cratebasehq/cratebase), README (MIT, self-hostable, one binary).

import { el, S, place, show, px, heavy, snappy, soft, prog, maskLine, fitWidth, statement, drawStatement, crateSVG, easeIn } from "../lib/kit.js";
import { clamp } from "../lib/motion.js";
import { addCue } from "../lib/timeline.js";

const INSTALL = "curl -fsSL https://cratebase.dev/install.sh | sh";

addCue(55.75, "swell", { dur: 0.25 });
addCue(56.0, "hit", { gain: 0.9 });
addCue(56.5, "whoosh", { dur: 0.4, gain: 0.5 });
addCue(57.0, "pop", { note: 69 });
addCue(58.0, "keys", { n: 16, dur: 0.75 });
addCue(59.0, "pop", { note: 74 });
addCue(59.5, "pop", { note: 77 });
addCue(60.0, "pop", { note: 81 });
addCue(62.0, "sub", { gain: 0.4 });

export default [
  {
    id: "cta",
    from: 56,
    to: 64,
    z: 3,
    build(root, L) {
      S(root, { background: "#0a1622" });
      const P = L.portrait;
      const cam = el("div", "abs", root);
      S(cam, { width: L.W + "px", height: L.H + "px", transformOrigin: P ? "50% 40%" : "30% 50%" });
      const U = (P ? 250 : 300) * L.u;
      const g = crateSVG(U);
      const Pj = g.P;
      const top = Pj([0, 0, 1])[1], bottom = Pj([1, 1, 0])[1];
      const svgW = U * 2 + 40, svgH = bottom - top + 40;
      const sw = (4 * L.u).toFixed(1);
      const crate = el("div", "abs", cam, `<svg width="${svgW}" height="${svgH}" viewBox="${-U - 20} ${top - 20} ${svgW} ${svgH}" style="overflow:visible">
        <path d="M${Pj([0.18, 0.18, 0]).join(" ")}L${Pj([1.18, 0.18, 0]).join(" ")}L${Pj([1.18, 1.18, 0]).join(" ")}L${Pj([0.18, 1.18, 0]).join(" ")}Z" fill="#000" opacity="0.45"/>
        <path d="${g.left}" fill="#cf703d" stroke="#eef1f4" stroke-width="${sw}" stroke-linejoin="round"/>
        <path d="${g.right}" fill="#ac623c" stroke="#eef1f4" stroke-width="${sw}" stroke-linejoin="round"/>
        <path d="${g.top}" fill="#ef7d3e" stroke="#eef1f4" stroke-width="${sw}" stroke-linejoin="round"/>
        ${g.br.map((d) => `<path d="${d}" fill="none" stroke="#eef1f4" stroke-width="${(3 * L.u).toFixed(1)}" stroke-linecap="round" opacity="0.55"/>`).join("")}
      </svg>`);
      const word = maskLine(cam, "disp", "Cratebase");
      const probe = el("div", "abs disp nowrap", root, "Cratebase");
      const wsize = P ? fitWidth(probe, L.W - 2 * L.m, 200) : fitWidth(probe, L.W * 0.5, 200);
      probe.remove();
      S(word.wrap, { fontSize: px(wsize), paddingBottom: px(wsize * 0.12) });
      const tag = statement(cam, ["A backend in one crate."]);
      const pill = el("div", "abs nowrap mono", cam);
      S(pill, { background: "#101f30", color: "#eef1f4", borderRadius: "999px", padding: "0.5em 0.9em", boxShadow: "0 0 0 2px rgba(238,241,244,.14)", fontWeight: 600 });
      const links = statement(cam, ["cratebase.dev", "github.com/cratebasehq/cratebase"], "stmt");
      const fine = statement(cam, P ? ["MIT licensed. Self-hostable.", "One binary."] : ["MIT licensed. Self-hostable. One binary."]);
      return { cam, crate, svgW, svgH, word, wsize, tag, pill, links, fine };
    },
    draw(t, s, L) {
      const P = L.portrait;
      const u = L.u;
      // crate: falls and lands on 56.0 (drawn from 55.6 on top of Proof's last beat)
      const fall = easeIn(prog(t, 55.6, 56.0));
      const squash = t >= 56 ? 1 - 0.05 * Math.exp(-(t - 56) * 10) : 1;
      const cx = P ? L.W * 0.5 : L.W * 0.79, cy = P ? L.H * 0.25 : L.H * 0.47;
      S(s.crate, { transform: `translate3d(${px(cx - s.svgW / 2)},${px(cy - s.svgH / 2 - (1 - fall) * L.H * 1.1)},0) scale(1,${squash.toFixed(4)})`, transformOrigin: "50% 100%" });
      // slow push-in 60–64 (constant-rate zoom, no settle: the film ends moving gently)
      const z = 1 + 0.06 * prog(t, 59.0, 64.0);
      S(s.cam, { transform: `scale(${z.toFixed(4)})` });
      const x = L.m;
      const wy = P ? L.H * 0.46 : L.H * 0.2;
      place(s.word.wrap, x, wy);
      show(s.word.wrap, t >= 56.4);
      S(s.word.inner, { transform: `translate3d(0,${((1 - heavy(t - 56.5)) * 125).toFixed(1)}%,0)` });
      const ty = wy + s.wsize * 0.98;
      drawStatement(s.tag, t, 57.0, 999, { x, y: ty, size: (P ? 74 : 92) * u, color: "#93a4b6" });
      // install pill types on 58.0 in 16th bursts
      const on = t >= 57.95;
      show(s.pill, on);
      if (on) {
        const n = Math.round(INSTALL.length * (Math.floor(clamp((t - 58.0) / 0.75) * 12) / 12));
        const caret = t < 59.2 && Math.floor(t * 4) % 2 === 0 ? '<span style="background:#ef7d3e">&nbsp;</span>' : "";
        s.pill.innerHTML = `<span style="color:#ef7d3e">$</span> ${INSTALL.slice(0, n)}${caret}`;
        S(s.pill, { fontSize: px((P ? 30 : 34) * u), transform: `translate3d(${px(x)},${px(ty + (P ? 120 : 150) * u)},0) scale(${snappy(t - 57.95).toFixed(3)})`, transformOrigin: "0 50%" });
      }
      drawStatement(s.links, t, 59.0, 999, { x, y: ty + (P ? 240 : 270) * u, size: (P ? 58 : 66) * u, color: "#eef1f4", stagger: 0.5, lh: 1.25 });
      drawStatement(s.fine, t, 60.0, 999, { x, y: ty + (P ? 420 : 440) * u, size: (P ? 40 : 46) * u, color: "#93a4b6" });
    },
  },
];
