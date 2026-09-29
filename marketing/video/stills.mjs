// node stills.mjs — one PNG per beat (the "stills" workflow gate), plus
// any extra timestamps passed as bare numeric args, e.g.:
//   node stills.mjs 12.3 45.0
// Reuses the exact same window.seek(t) contract as render.mjs, just
// without the ffmpeg pipe — faster for the "one key frame per shot,
// look at it" loop than a full render.

import { chromium } from "playwright";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const OUT_DIR = path.join(__dirname, "out", "stills");
mkdirSync(OUT_DIR, { recursive: true });

// Beat midpoints from docs/shotlist.md's refined chapter table.
const BEATS = [
  { name: "01-hook", t: 3.2 },
  { name: "02-one-binary", t: 7.0 },
  { name: "03-data", t: 15.0 },
  { name: "04-auth", t: 24.0 },
  { name: "05-email", t: 32.0 },
  { name: "06-superpowers", t: 43.0 },
  { name: "07-proof", t: 54.0 },
  { name: "08-cta", t: 65.0 },
];

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png" };

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
  const extra = process.argv.slice(2).map(Number).filter((n) => !Number.isNaN(n));
  const targets = extra.length ? extra.map((t, i) => ({ name: `extra-${i}-${t}s`, t })) : BEATS;

  const server = await startServer();
  const port = server.address().port;
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  await page.goto(`http://127.0.0.1:${port}/index.html`);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(() => typeof window.seek === "function");
  const stage = page.locator("#stage");

  for (const { name, t } of targets) {
    await page.evaluate((t) => window.seek(t), t);
    const file = path.join(OUT_DIR, `${name}.png`);
    await stage.screenshot({ path: file });
    console.log(`still ${name} (t=${t}s) -> ${file}`);
  }

  await browser.close();
  server.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
