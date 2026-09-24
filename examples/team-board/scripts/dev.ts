#!/usr/bin/env bun
// `bun run dev` — the one-command entry point: builds the server binary
// if it isn't already built, starts it against this example's own data
// directory, provisions it (superuser, settings.teams.enabled,
// schema.json, seed data, TypeScript types), then starts Vite. Ctrl-C
// stops both child processes.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const APP_DIR = path.resolve(HERE, "..");
const REPO_ROOT = path.resolve(APP_DIR, "../..");

const HOST = process.env.CRATEBASE_HOST ?? "127.0.0.1";
const PORT = process.env.CRATEBASE_PORT ?? "8090";
const CRATEBASE_URL = `http://${HOST}:${PORT}`;
const CB_SETUP_TOKEN = process.env.CB_SETUP_TOKEN ?? "team-board-dev-setup-token";
const CB_DATA_DIR = process.env.CB_DATA_DIR ?? "./pb_data";
const BUILT_BIN = path.join(REPO_ROOT, "target", "debug", "cratebase");
const CRATEBASE_BIN = process.env.CRATEBASE_BIN ?? (existsSync(BUILT_BIN) ? BUILT_BIN : "cratebase");

const children: ReturnType<typeof spawn>[] = [];
let shuttingDown = false;

function run(cmd: string, args: string[], opts: Parameters<typeof spawn>[2] = {}): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: "inherit", cwd: APP_DIR, ...opts });
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${cmd} ${args.join(" ")} exited with code ${code}`));
    });
    child.on("error", reject);
  });
}

function runBackground(name: string, cmd: string, args: string[], opts: Parameters<typeof spawn>[2] = {}) {
  const child = spawn(cmd, args, { stdio: "inherit", cwd: APP_DIR, ...opts });
  children.push(child);
  child.on("exit", (code, signal) => {
    if (!shuttingDown) {
      console.error(`==> ${name} exited unexpectedly (code=${code} signal=${signal}); shutting down.`);
      shutdown(1);
    }
  });
  return child;
}

function shutdown(code: number) {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) child.kill("SIGTERM");
  setTimeout(() => process.exit(code), 300);
}
process.on("SIGINT", () => shutdown(0));
process.on("SIGTERM", () => shutdown(0));

async function waitForHealth() {
  console.log(`==> Waiting for ${CRATEBASE_URL}/api/health ...`);
  for (let i = 0; i < 60; i++) {
    try {
      const res = await fetch(`${CRATEBASE_URL}/api/health`);
      if (res.ok) {
        console.log("==> Server is up.");
        return;
      }
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`${CRATEBASE_URL} never became healthy`);
}

async function main() {
  if (!existsSync(CRATEBASE_BIN) && CRATEBASE_BIN === BUILT_BIN) {
    console.log("==> Building cratebase-server (first run only; CARGO_BUILD_JOBS=3) ...");
    await run("cargo", ["build", "-p", "cratebase-server"], {
      cwd: REPO_ROOT,
      // Capped deliberately — parallel/high-job-count Rust builds have
      // crashed the dev VM here before (memory pressure). Override with
      // CARGO_BUILD_JOBS if your machine can handle more.
      env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    });
  }

  console.log(`==> Starting Cratebase (${CRATEBASE_BIN}) at ${CRATEBASE_URL} ...`);
  runBackground("cratebase serve", CRATEBASE_BIN, [
    "serve",
    "--http", `${HOST}:${PORT}`,
    "--dir", CB_DATA_DIR,
    "--dev",
  ], {
    env: {
      ...process.env,
      CB_SETUP_TOKEN,
      // Keeps src/cratebase-types.d.ts in sync with any further schema
      // edit made live through the dashboard while this dev server runs.
      CB_TYPEGEN_OUT: path.join(APP_DIR, "src", "cratebase-types.d.ts"),
    },
  });

  await waitForHealth();

  const setupEnv = {
    ...process.env,
    CRATEBASE_URL,
    CB_SETUP_TOKEN,
    CRATEBASE_BIN,
    CB_DATA_DIR,
  };
  console.log("==> Running setup (superuser, teams, schema, types) ...");
  await run("bash", ["scripts/setup.sh"], { env: setupEnv });

  console.log("==> Seeding demo data (skipped if already present) ...");
  await run("bun", ["run", "scripts/seed.ts"], { env: setupEnv });

  console.log("==> Starting Vite ...");
  runBackground("vite", "bunx", ["vite"], { env: { ...process.env, CRATEBASE_URL } });

  console.log("\n==> team-board is running. Dashboard: " + CRATEBASE_URL + "/_/  (superuser: admin@example.com / changeme123)");
  console.log("    Demo users: alice@example.com / bob@example.com / carol@example.com, password: password123\n");
}

main().catch((err) => {
  console.error(err);
  shutdown(1);
});
