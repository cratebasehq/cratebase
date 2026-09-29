// Node-side helpers shared by render.mjs and stills.mjs: a static file
// server for marketing/video/ (module scripts + images need http://, not
// file://) and a page opener that waits for fonts + image decodes.

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MIME = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".png": "image/png", ".jpg": "image/jpeg", ".json": "application/json", ".woff2": "font/woff2", ".svg": "image/svg+xml" };

export function startServer() {
  return new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      try {
        const urlPath = decodeURIComponent(req.url.split("?")[0]);
        const filePath = path.join(ROOT, urlPath === "/" ? "/index.html" : urlPath);
        if (!filePath.startsWith(ROOT)) throw new Error("outside root");
        const buf = await readFile(filePath);
        res.writeHead(200, { "Content-Type": MIME[path.extname(filePath)] ?? "application/octet-stream", "Cache-Control": "max-age=3600" });
        res.end(buf);
      } catch {
        res.writeHead(404);
        res.end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

export const arg = (k, d) => {
  const i = process.argv.indexOf("--" + k);
  return i > 0 ? process.argv[i + 1] : d;
};

/** Open the film page in a fresh context and wait until window.__ready. */
export async function openFilm(browser, port, W, H, extra = "") {
  const context = await browser.newContext({ viewport: { width: W, height: H }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error("[page error]", e.message));
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") console.error("[console]", m.text());
  });
  await page.goto(`http://127.0.0.1:${port}/index.html?w=${W}&h=${H}${extra}`);
  await page.waitForFunction(() => window.__ready !== undefined);
  await page.evaluate(() => window.__ready);
  const cdp = await context.newCDPSession(page);
  const shoot = async (fmt = "png", quality = 95) => {
    const r = await cdp.send("Page.captureScreenshot", {
      format: fmt,
      ...(fmt === "jpeg" ? { quality } : {}),
      clip: { x: 0, y: 0, width: W, height: H, scale: 1 },
      optimizeForSpeed: true,
      captureBeyondViewport: false,
    });
    return Buffer.from(r.data, "base64");
  };
  return { context, page, shoot };
}
