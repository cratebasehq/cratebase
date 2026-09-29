// Cratebase launch film — render contract: window.seek(t) paints frame t.
// Pure function of time: no CSS transitions, no timers, no state carried
// between frames, no Math.random. Stage size comes from the URL
// (?w=1920&h=1080 or ?w=1080&h=1920) and every scene lays itself out from
// layout(W, H) — the vertical cut is a reflow, never a crop.

import { layout, S } from "./lib/kit.js";
import { DUR } from "./lib/timeline.js";
import { spring } from "./lib/motion.js";

const q = new URLSearchParams(location.search);
const W = Number(q.get("w") || 1920);
const H = Number(q.get("h") || 1080);
const only = q.get("only"); // dev: build a single chapter

const stage = document.getElementById("stage");
const world = document.getElementById("world");
S(stage, { width: W + "px", height: H + "px" });
const L = layout(W, H);

const MODULES = ["hook", "binary", "data", "auth", "email", "superpowers", "proof", "cta"];

const scenes = [];
async function boot() {
  await document.fonts.load('800 100px "Bricolage"');
  await Promise.all(['500 40px "RHMono"', '500 40px "Plex"', '500 40px "JBMono"', '500 40px "Inter"', '600 40px "Archivo"'].map((f) => document.fonts.load(f)));
  await document.fonts.ready;
  for (const name of MODULES) {
    if (only && only !== name) continue;
    let mod;
    try {
      mod = await import(`./scenes/${name}.js`);
    } catch (e) {
      if (String(e).includes("Failed to fetch")) continue; // chapter not written yet
      throw e;
    }
    for (const def of mod.default) {
      const root = document.createElement("div");
      root.className = "scene";
      root.dataset.scene = def.id;
      S(root, { zIndex: String(def.z ?? 1) });
      world.appendChild(root);
      const inst = { ...def, root };
      // build while visible so text can be measured (fitWidth), then hide
      inst.state = (await def.build?.(root, L)) ?? {};
      S(root, { display: "none" });
      scenes.push(inst);
    }
  }
  const imgs = [...document.images];
  await Promise.all(imgs.map((i) => i.decode().catch(() => {})));
}

// Impact dips on the two hero hits: the whole world drops a few px and
// settles on the heavy spring (the only "shake" in the film).
const HITS = [4.0, 56.0];
function dip(t) {
  let y = 0;
  for (const h of HITS) {
    const d = t - h;
    if (d >= 0 && d < 1.2) y += 16 * L.u * Math.exp(-d * 7) * Math.sin(d * 18 + 0.2) * (1 - spring(d, 150, 34) * 0.3);
  }
  return y;
}

window.seek = (t) => {
  t = Math.max(0, Math.min(DUR - 1e-6, t));
  for (const s of scenes) {
    const on = t >= s.from && t < s.to;
    S(s.root, { display: on ? "" : "none" });
    if (on) s.draw(t, s.state, L);
  }
  S(world, { transform: `translate3d(0,${dip(t).toFixed(2)}px,0)` });
  return true;
};

window.__ready = boot().then(() => {
  window.seek(Number(q.get("t") || 0));
  return true;
});

// Live preview in a normal browser tab (never during headless render).
if (!navigator.webdriver && q.get("play") !== null) {
  window.__ready.then(() => {
    const t0 = performance.now() - Number(q.get("t") || 0) * 1000;
    const loop = () => {
      window.seek(((performance.now() - t0) / 1000) % DUR);
      requestAnimationFrame(loop);
    };
    loop();
  });
}
