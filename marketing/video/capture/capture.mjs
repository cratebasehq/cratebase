// Capture real Cratebase dashboard screens for the launch film.
//
// Runs inside the Docker image (see ../Dockerfile) with --network host so
// it can reach `cratebase dev` listening on the host's localhost:8090 —
// this machine's own Chromium is missing system libs and there's no sudo.
//
// Auth: rather than scripting the login form for every shot, this signs
// in once via the same REST call the dashboard itself makes
// (`/api/collections/_superusers/auth-with-password`) and writes the
// result straight into `localStorage["cratebase_auth"]` in the shape
// `LocalAuthStore` expects (`{token, record}` — see
// `sdk/js/client/src/auth-store.ts`), then reloads. Every subsequent
// screenshot is a fresh page load already signed in.
//
// Never redraws the product UI from imagination: every PNG here is a
// pixel-for-pixel capture of the real v0.4.0 dashboard running against
// the demo schema/seed described in docs/shotlist.md.

import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const OUT_DIR = process.env.CAPTURE_OUT ?? path.join(__dirname, "shots");
const BASE_URL = process.env.CRATEBASE_URL ?? "http://127.0.0.1:8090";
const EMAIL = process.env.CRATEBASE_SUPERUSER_EMAIL;
const PASSWORD = process.env.CRATEBASE_SUPERUSER_PASSWORD;

if (!EMAIL || !PASSWORD) {
  console.error(
    "Set CRATEBASE_SUPERUSER_EMAIL and CRATEBASE_SUPERUSER_PASSWORD " +
      "(printed once by `cratebase dev` — see README.md).",
  );
  process.exit(1);
}

mkdirSync(OUT_DIR, { recursive: true });

// One row per shot in docs/shotlist.md. `tab`/`search` become the route's
// query string (TanStack Router search params — see
// web/admin/src/routes/collection.tsx's `CollectionSearch`).
const SHOTS = [
  { name: "01-dashboard-home", path: "/_/" },
  { name: "02-collections-places-records", path: "/_/collections/places" },
  { name: "03-collections-places-schema-rules", path: "/_/collections/places?tab=schema" },
  { name: "04-collections-places-api-docs", path: "/_/collections/places?tab=api" },
  { name: "05-settings-index", path: "/_/settings" },
  // OAuth/OIDC/TOTP providers aren't under Settings → Auth & security
  // (that page is superusers/sessions/API keys/network) — they're the
  // `users` auth collection's own "Auth options" panel, on its schema
  // tab (web/admin/src/components/collections/auth-options-editor.tsx).
  { name: "06-users-auth-options", path: "/_/collections/users?tab=schema", scrollTo: "OAuth2" },
  { name: "07-settings-email-templates", path: "/_/settings/email?tab=templates" },
  { name: "08-settings-email-template-editor", path: "/_/settings/email-templates/{{welcomeTemplateId}}" },
  { name: "09-settings-rpc", path: "/_/settings/rpc" },
  { name: "10-settings-mail-inbox", path: "/_/settings/mail-inbox" },
  { name: "11-collections-posts-records-search", path: "/_/collections/posts" },
];

async function findWelcomeTemplateId(request) {
  const res = await request.get(
    `${BASE_URL}/api/collections/_emailTemplates/records?filter=${encodeURIComponent('key = "welcome"')}`,
    { headers: { Authorization: `Bearer ${global.__token}` } },
  );
  const body = await res.json();
  const item = body.items?.[0];
  if (!item) throw new Error('No "welcome" email template found — did capture/seed.sh run?');
  return item.id;
}

async function main() {
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1600, height: 1000 },
    deviceScaleFactor: 2, // high-DPI, per the brief
  });
  const page = await context.newPage();
  const request = context.request;

  // Sign in exactly the way the dashboard does, then seed localStorage
  // so every navigation below loads already authenticated.
  const authRes = await request.post(`${BASE_URL}/api/collections/_superusers/auth-with-password`, {
    data: { identity: EMAIL, password: PASSWORD },
  });
  if (!authRes.ok()) {
    throw new Error(`Superuser auth failed: ${authRes.status()} ${await authRes.text()}`);
  }
  const auth = await authRes.json();
  global.__token = auth.token;

  await page.goto(`${BASE_URL}/_/`);
  await page.evaluate(
    ({ token, record }) => {
      localStorage.setItem("cratebase_auth", JSON.stringify({ token, record }));
    },
    { token: auth.token, record: auth.record },
  );

  const welcomeTemplateId = await findWelcomeTemplateId(request);
  const manifest = [];

  for (const shot of SHOTS) {
    const url = shot.path.replace("{{welcomeTemplateId}}", welcomeTemplateId);
    try {
      // Not "networkidle": the dashboard opens a long-lived SSE realtime
      // subscription on most screens (GET /api/realtime), which never
      // goes idle and would time this out. "load" + a settle delay is
      // the reliable option against this app. The email template editor
      // additionally loads a third-party rich editor into an iframe
      // (React Email Editor), so it gets a longer settle delay.
      await page.goto(`${BASE_URL}${url}`, { waitUntil: "load", timeout: 30000 });
      const settleMs = shot.name.includes("template-editor") ? 4000 : 1200;
      await page.waitForTimeout(settleMs);
      if (shot.scrollTo) {
        await page
          .getByText(shot.scrollTo, { exact: false })
          .first()
          .scrollIntoViewIfNeeded({ timeout: 5000 })
          .catch(() => {});
        await page.waitForTimeout(300);
      }
      const file = path.join(OUT_DIR, `${shot.name}.png`);
      await page.screenshot({ path: file, fullPage: false, timeout: 45000 });
      manifest.push({ name: shot.name, path: shot.path, file: path.basename(file) });
      console.log(`captured ${shot.name} -> ${file}`);
    } catch (err) {
      // One flaky shot (a slow third-party iframe, a transient SSE
      // reconnect) should never sink the rest of the capture run — log
      // it in the manifest as failed and keep going.
      console.error(`FAILED ${shot.name}: ${err.message}`);
      manifest.push({ name: shot.name, path: shot.path, failed: true, error: err.message });
    }
  }

  writeFileSync(path.join(OUT_DIR, "manifest.json"), JSON.stringify(manifest, null, 2));
  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
