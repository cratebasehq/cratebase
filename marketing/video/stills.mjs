// node stills.mjs [--w 1920 --h 1080] [--dir out/stills] [--prefix s] t1 t2 ...
//   or: node stills.mjs --every 0.5 [--start 0 --end 64]
//
// Same window.seek(t) contract as render.mjs, without ffmpeg — the fast
// "look at one frame per beat" loop for the stills gate and critique.

import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { startServer, arg, openFilm, ROOT } from "./lib/serve.mjs";

const W = Number(arg("w", 1920));
const H = Number(arg("h", 1080));
const DIR = path.resolve(ROOT, arg("dir", "out/stills"));
const PREFIX = arg("prefix", "s");
const EVERY = Number(arg("every", 0));

async function main() {
  mkdirSync(DIR, { recursive: true });
  const flagVals = new Set();
  process.argv.forEach((a, i) => { if (a.startsWith("--")) flagVals.add(i + 1); });
  let times = process.argv.slice(2).filter((a, i) => !a.startsWith("--") && !flagVals.has(i + 2)).map(Number).filter((n) => !Number.isNaN(n));
  if (EVERY) {
    const s = Number(arg("start", 0)), e = Number(arg("end", 64));
    times = [];
    for (let t = s; t < e - 1e-9; t += EVERY) times.push(+t.toFixed(4));
  }
  const server = await startServer();
  const browser = await chromium.launch();
  const { page, shoot } = await openFilm(browser, server.address().port, W, H);
  const t0 = Date.now();
  for (const t of times) {
    await page.evaluate((t) => window.seek(t), t);
    const file = path.join(DIR, `${PREFIX}-${t.toFixed(2).padStart(6, "0")}.png`);
    writeFileSync(file, await shoot("png"));
  }
  console.log(`${times.length} stills -> ${path.relative(ROOT, DIR)} (${((Date.now() - t0) / times.length).toFixed(0)} ms/frame)`);
  await browser.close();
  server.close();
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
