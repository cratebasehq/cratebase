// One-off capture of the real sign-in card from examples/team-board's
// AuthScreen.tsx — the actual shipped tabbed "Password / Email code /
// Magic link" UI, used for docs/shotlist.md shot 4.2 instead of an
// invented card, since this real example ships exactly that morph.
//
// Not part of the main capture.mjs pipeline because it targets a
// separate app/server (examples/team-board's own Vite dev server, not
// the film's demo `cratebase dev` instance) — see
// examples/team-board/README.md for how to start it. Run inside the
// same Docker image as capture.mjs, --network host.

import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const OUT_DIR = process.env.CAPTURE_OUT ?? path.join(__dirname, "shots");
const APP_URL = process.env.TEAM_BOARD_URL ?? "http://127.0.0.1:5175";

mkdirSync(OUT_DIR, { recursive: true });

const TABS = [
  { tab: "Password", name: "12-team-board-auth-password" },
  { tab: "Email code", name: "13-team-board-auth-otp" },
  { tab: "Magic link", name: "14-team-board-auth-magic-link" },
];

async function main() {
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1200, height: 900 },
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  await page.goto(APP_URL, { waitUntil: "load" });
  await page.waitForTimeout(800);

  for (const { tab, name } of TABS) {
    await page.getByRole("button", { name: tab, exact: true }).click();
    await page.waitForTimeout(300);
    const file = path.join(OUT_DIR, `${name}.png`);
    await page.screenshot({ path: file });
    console.log(`captured ${name} -> ${file}`);
  }

  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
