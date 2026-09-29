// Capture real Cratebase dashboard screens for the launch film.
//
// Runs inside the Docker image (see ../Dockerfile) with --network host so
// it can reach a `cratebase dev` instance on the host's loopback — this
// machine's own Chromium is missing system libs and there's no sudo.
//
// v2 (art-direction rework): the film's virtual camera pushes in on single
// details (a rule expression, a `{{user.name}}` variable, one field row),
// so every capture is taken at deviceScaleFactor 4 on a narrow 1280px
// viewport — the framed detail must still be ≥ 1:1 texels when a 40px
// dashboard glyph is blown up to ~56px on a 1080p frame. Each shot also
// writes `<name>.boxes.json`: the on-screen rect (CSS px) of every input,
// button and text run, so camera framings and callouts in main.js anchor
// to measured element positions instead of eyeballed pixel guesses.
//
// Auth: signs in once via the same REST call the dashboard makes
// (`/api/collections/_superusers/auth-with-password`) and writes the result
// into `localStorage["cratebase_auth"]` in the shape `LocalAuthStore`
// expects (`{token, record}` — `sdk/js/client/src/auth-store.ts`).
//
// Never redraws the product UI: every PNG is a pixel capture of the real
// v0.4.0 dashboard running the demo schema from capture/seed.sh.

import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const OUT_DIR = process.env.CAPTURE_OUT ?? path.join(__dirname, "shots");
const BASE_URL = process.env.CRATEBASE_URL ?? "http://127.0.0.1:8090";
const EMAIL = process.env.CRATEBASE_SUPERUSER_EMAIL;
const PASSWORD = process.env.CRATEBASE_SUPERUSER_PASSWORD;
const DPR = Number(process.env.CAPTURE_DPR ?? 4);
const VIEW_W = Number(process.env.CAPTURE_WIDTH ?? 1280);

if (!EMAIL || !PASSWORD) {
  console.error(
    "Set CRATEBASE_SUPERUSER_EMAIL and CRATEBASE_SUPERUSER_PASSWORD " +
      "(printed once by `cratebase dev` — see README.md).",
  );
  process.exit(1);
}

mkdirSync(OUT_DIR, { recursive: true });

// One row per camera subject in docs/shotlist.md. `h` is the viewport
// height for that shot (tall pages give the camera somewhere to travel);
// `steps` run after load (click a row, scroll a pane) before the capture.
const SHOTS = [
  { name: "hd-overview", path: "/_/", h: 900 },
  { name: "hd-places-schema", path: "/_/collections/places?tab=schema", h: 1560 },
  { name: "hd-places-records", path: "/_/collections/places", h: 820 },
  { name: "hd-places-api", path: "/_/collections/places?tab=api", h: 1200 },
  { name: "hd-email-templates", path: "/_/settings/email?tab=templates", h: 820 },
  { name: "hd-email-editor", path: "/_/settings/email-templates/{{welcomeTemplateId}}", h: 900, settle: 4000 },
  {
    name: "hd-mail-inbox",
    path: "/_/settings/mail-inbox",
    h: 900,
    // capture/seed-film.sh sends one real `welcome` mail through
    // POST /api/mails/send; open it so the reading pane shows the render.
    steps: async (page) => {
      await page.getByText("Welcome", { exact: false }).first().click({ timeout: 5000 });
      await page.waitForTimeout(1500);
    },
  },
];

async function findWelcomeTemplateId(request, token) {
  const res = await request.get(
    `${BASE_URL}/api/collections/_emailTemplates/records?filter=${encodeURIComponent('key = "welcome"')}`,
    { headers: { Authorization: `Bearer ${token}` } },
  );
  const body = await res.json();
  const item = body.items?.[0];
  if (!item) throw new Error('No "welcome" email template found — did capture/seed.sh run?');
  return item.id;
}

// Every visible input/textarea (with its value), button, and text run, in
// CSS px relative to the viewport. Text runs are measured per text node
// with a Range, so a single token like `owner = @request.auth.id` inside a
// larger block still gets its own rect.
function dumpBoxes() {
  const out = [];
  const vh = window.innerHeight;
  const vw = window.innerWidth;
  const keep = (r) => r.width >= 1 && r.height >= 1 && r.bottom > 0 && r.top < vh && r.right > 0 && r.left < vw;
  const rect = (r) => ({ x: +r.x.toFixed(1), y: +r.y.toFixed(1), w: +r.width.toFixed(1), h: +r.height.toFixed(1) });
  for (const el of document.querySelectorAll("input, textarea")) {
    const r = el.getBoundingClientRect();
    if (keep(r)) out.push({ kind: el.tagName.toLowerCase(), text: (el.value || el.placeholder || "").slice(0, 200), ...rect(r) });
  }
  for (const el of document.querySelectorAll("button, a, [role=tab], [role=switch]")) {
    const r = el.getBoundingClientRect();
    const text = (el.innerText || el.getAttribute("aria-label") || "").trim();
    if (keep(r)) out.push({ kind: "button", text: text.slice(0, 200), ...rect(r) });
  }
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  while (walker.nextNode()) {
    const n = walker.currentNode;
    const text = n.textContent.replace(/\s+/g, " ").trim();
    if (!text) continue;
    range.selectNodeContents(n);
    const r = range.getBoundingClientRect();
    if (keep(r)) out.push({ kind: "text", text: text.slice(0, 200), ...rect(r) });
  }
  return out;
}

async function main() {
  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: VIEW_W, height: 900 }, deviceScaleFactor: DPR });
  const page = await context.newPage();
  const request = context.request;

  const authRes = await request.post(`${BASE_URL}/api/collections/_superusers/auth-with-password`, {
    data: { identity: EMAIL, password: PASSWORD },
  });
  if (!authRes.ok()) throw new Error(`Superuser auth failed: ${authRes.status()} ${await authRes.text()}`);
  const auth = await authRes.json();

  await page.goto(`${BASE_URL}/_/`);
  await page.evaluate(({ token, record }) => {
    localStorage.setItem("cratebase_auth", JSON.stringify({ token, record }));
  }, auth);

  const welcomeTemplateId = await findWelcomeTemplateId(request, auth.token);
  const manifest = [];

  for (const shot of SHOTS) {
    const url = shot.path.replace("{{welcomeTemplateId}}", welcomeTemplateId);
    try {
      await page.setViewportSize({ width: VIEW_W, height: shot.h });
      // Not "networkidle": the dashboard holds a long-lived SSE realtime
      // subscription (GET /api/realtime) that never goes idle.
      await page.goto(`${BASE_URL}${url}`, { waitUntil: "load", timeout: 30000 });
      await page.waitForTimeout(shot.settle ?? 1500);
      if (shot.steps) await shot.steps(page);
      const file = path.join(OUT_DIR, `${shot.name}.png`);
      await page.screenshot({ path: file, fullPage: false, timeout: 60000 });
      const boxes = await page.evaluate(dumpBoxes);
      writeFileSync(
        path.join(OUT_DIR, `${shot.name}.boxes.json`),
        JSON.stringify({ dpr: DPR, width: VIEW_W, height: shot.h, boxes }, null, 1),
      );
      manifest.push({ name: shot.name, path: shot.path, file: path.basename(file), dpr: DPR, width: VIEW_W, height: shot.h });
      console.log(`captured ${shot.name} (${VIEW_W}x${shot.h} @${DPR}x, ${boxes.length} boxes)`);
    } catch (err) {
      console.error(`FAILED ${shot.name}: ${err.message}`);
      manifest.push({ name: shot.name, path: shot.path, failed: true, error: err.message });
    }
  }

  writeFileSync(path.join(OUT_DIR, "manifest-hd.json"), JSON.stringify(manifest, null, 2));
  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
