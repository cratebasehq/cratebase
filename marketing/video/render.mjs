// node render.mjs [--fps 30] [--dur 66] [--sub 1] [--start 0] [--end DUR]
//                 [--out out/animatic.mp4] [--w 1920] [--h 1080]
//
// Route A from motion-course.md: window.seek(t) paints frame t, Playwright
// calls it frame by frame in headless Chrome, ffmpeg encodes the piped
// PNGs. `--sub` > 1 averages that many subframes per output frame for
// motion blur (course's tmix pattern) — default 1 (off) for the
// animatic/stills passes; the polish pass turns it on.
//
// Serves marketing/video/ over plain HTTP (not file://) so main.js's ES
// module import and the capture/shots/*.png <img> loads work the same
// way a browser expects — file:// blocks module scripts under some
// Chromium security policies.
//
// Runs inside Docker (see Dockerfile) — this machine's own Chromium is
// missing system libs and there's no sudo.

import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { existsSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const arg = (k, d) => {
  const i = process.argv.indexOf("--" + k);
  return i > 0 ? process.argv[i + 1] : d;
};
const FPS = Number(arg("fps", 30));
const DUR = Number(arg("dur", 66));
const SUB = Number(arg("sub", 1));
const START = Number(arg("start", 0));
const END = Number(arg("end", DUR));
const OUT = arg("out", "out/animatic.mp4");
const W = Number(arg("w", 1920));
const H = Number(arg("h", 1080));

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".json": "application/json" };

function startServer() {
  return new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      try {
        const urlPath = decodeURIComponent(req.url.split("?")[0]);
        const filePath = path.join(__dirname, urlPath === "/" ? "/index.html" : urlPath);
        const buf = await readFile(filePath);
        res.writeHead(200, { "Content-Type": MIME[path.extname(filePath)] ?? "application/octet-stream" });
        res.end(buf);
      } catch {
        res.writeHead(404);
        res.end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

async function main() {
  mkdirSync(path.dirname(path.join(__dirname, OUT)), { recursive: true });

  const server = await startServer();
  const port = server.address().port;

  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 1 });
  await page.goto(`http://127.0.0.1:${port}/index.html`);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(() => typeof window.seek === "function");

  const stage = page.locator("#stage");

  const outPath = path.join(__dirname, OUT);
  const vf = SUB > 1
    ? `tmix=frames=${SUB},select='eq(mod(n\\,${SUB})\\,${SUB - 1})',setpts=N/${FPS}/TB`
    : "setpts=N/" + FPS + "/TB";

  const ff = spawn(
    "ffmpeg",
    [
      "-y",
      "-f", "image2pipe",
      "-framerate", String(FPS * SUB),
      "-i", "-",
      "-vf", vf,
      "-r", String(FPS),
      "-c:v", "libx264",
      "-crf", "18",
      "-pix_fmt", "yuv420p",
      outPath,
    ],
    { stdio: ["pipe", "inherit", "inherit"] },
  );

  const totalSeconds = END - START;
  const totalFrames = Math.round(totalSeconds * FPS * SUB);
  console.log(`Rendering ${totalSeconds}s (${START}s..${END}s) at ${FPS}fps x${SUB} sub -> ${OUT}`);

  for (let i = 0; i < totalFrames; i++) {
    const t = START + i / (FPS * SUB);
    await page.evaluate((t) => window.seek(t), t);
    const png = await stage.screenshot({ type: "png" });
    if (!ff.stdin.write(png)) await new Promise((r) => ff.stdin.once("drain", r));
    if (i % (FPS * SUB) === 0) console.log(`  ${(i / (FPS * SUB)).toFixed(0)}s / ${totalSeconds}s`);
  }
  ff.stdin.end();
  await new Promise((r) => ff.on("close", r));

  await browser.close();
  server.close();
  console.log(`Done: ${OUT}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
