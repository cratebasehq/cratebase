// node render.mjs --w 1920 --h 1080 --fps 60 [--start 0] [--end 64]
//                 [--workers 4] [--sub 1] [--fmt png|jpeg] [--out out/master-16x9.mp4]
//
// Route A from motion-course.md: window.seek(t) paints frame t, headless
// Chromium captures it, ffmpeg encodes. The frame range is split into
// `--workers` contiguous chunks, each rendered by its own browser context
// (its own renderer process) into a near-lossless segment; segments are
// concatenated losslessly at the end. Final delivery encodes (size-tuned
// H.264, VP9, audio mux) happen in encode.sh from this intermediate.
//
// `--sub N` renders N subframes per output frame and averages them
// (tmix) for motion blur.
//
// Runs inside Docker (see Dockerfile): the host Chromium lacks system libs.

import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import path from "node:path";
import { startServer, arg, openFilm, ROOT } from "./lib/serve.mjs";

const W = Number(arg("w", 1920));
const H = Number(arg("h", 1080));
const FPS = Number(arg("fps", 60));
const SUB = Number(arg("sub", 1));
const START = Number(arg("start", 0));
const END = Number(arg("end", 64));
const WORKERS = Number(arg("workers", 4));
const FMT = arg("fmt", "jpeg");
const OUT = path.resolve(ROOT, arg("out", `out/master-${W}x${H}.mp4`));
const CRF = arg("crf", "10");

function encoder(file, rate) {
  const vf = SUB > 1 ? `tmix=frames=${SUB},select='eq(mod(n\\,${SUB})\\,${SUB - 1})',setpts=N/${FPS}/TB` : `setpts=N/${FPS}/TB`;
  return spawn(
    "ffmpeg",
    ["-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", String(rate), "-i", "-", "-vf", vf, "-r", String(FPS),
      "-c:v", "libx264", "-preset", "veryfast", "-crf", CRF, "-pix_fmt", "yuv420p", file],
    { stdio: ["pipe", "inherit", "inherit"] },
  );
}

async function renderChunk(browser, port, idx, f0, f1, partFile) {
  const { context, page, shoot } = await openFilm(browser, port, W, H);
  const rate = FPS * SUB;
  const ff = encoder(partFile, rate);
  const closed = new Promise((r) => ff.on("close", r));
  const t0 = Date.now();
  for (let f = f0; f < f1; f++) {
    for (let s = 0; s < SUB; s++) {
      const t = START + (f + (s + 1 - SUB) / SUB + (SUB > 1 ? 0.5 / SUB : 0)) / FPS;
      await page.evaluate((t) => window.seek(t), Math.max(START, t));
      const img = await shoot(FMT);
      if (!ff.stdin.write(img)) await new Promise((r) => ff.stdin.once("drain", r));
    }
    if ((f - f0) % (FPS * 2) === 0) console.log(`  worker ${idx}: frame ${f - f0}/${f1 - f0} (${((Date.now() - t0) / 1000).toFixed(0)}s)`);
  }
  ff.stdin.end();
  await closed;
  await context.close();
}

async function main() {
  mkdirSync(path.dirname(OUT), { recursive: true });
  const partsDir = OUT + ".parts";
  rmSync(partsDir, { recursive: true, force: true });
  mkdirSync(partsDir, { recursive: true });

  const server = await startServer();
  const port = server.address().port;
  const browser = await chromium.launch({ args: ["--disable-gpu-vsync", "--disable-frame-rate-limit"] });

  const total = Math.round((END - START) * FPS);
  const per = Math.ceil(total / WORKERS);
  const t0 = Date.now();
  console.log(`Rendering ${W}x${H} ${START}-${END}s @${FPS}fps x${SUB} sub, ${total} frames, ${WORKERS} workers -> ${path.relative(ROOT, OUT)}`);
  const parts = [];
  const jobs = [];
  for (let i = 0; i < WORKERS; i++) {
    const f0 = i * per, f1 = Math.min(total, (i + 1) * per);
    if (f0 >= f1) break;
    const file = path.join(partsDir, `part-${String(i).padStart(2, "0")}.mp4`);
    parts.push(file);
    jobs.push(renderChunk(browser, port, i, f0, f1, file));
  }
  await Promise.all(jobs);
  await browser.close();
  server.close();

  const list = path.join(partsDir, "list.txt");
  writeFileSync(list, parts.map((p) => `file '${p}'`).join("\n"));
  await new Promise((resolve, reject) => {
    const ff = spawn("ffmpeg", ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", list, "-c", "copy", OUT], { stdio: "inherit" });
    ff.on("close", (c) => (c === 0 ? resolve() : reject(new Error("concat failed"))));
  });
  rmSync(partsDir, { recursive: true, force: true });
  const secs = (Date.now() - t0) / 1000;
  console.log(`Done: ${path.relative(ROOT, OUT)} in ${secs.toFixed(1)}s (${(total / secs).toFixed(2)} fps)`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
