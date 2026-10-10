import { spawn } from "node:child_process";

import { PACKAGE_MANAGERS, type PackageManager } from "./templates.js";

export interface RunResult {
  code: number;
  /** Combined stdout + stderr, for showing after a failure. */
  output: string;
}

/** Runs a command with captured output (so a spinner can stay on screen). Never rejects. */
export function run(command: string, args: string[], cwd: string): Promise<RunResult> {
  return new Promise((resolve) => {
    let output = "";
    const child = spawn(command, args, {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
      shell: process.platform === "win32",
    });
    child.stdout?.on("data", (chunk: Buffer) => (output += chunk.toString()));
    child.stderr?.on("data", (chunk: Buffer) => (output += chunk.toString()));
    child.on("close", (code) => resolve({ code: code ?? 1, output }));
    child.on("error", (err) => resolve({ code: 1, output: output + String(err) }));
  });
}

/** True when `command --version` exits 0. */
export async function commandExists(command: string, cwd = process.cwd()): Promise<boolean> {
  return (await run(command, ["--version"], cwd)).code === 0;
}

/** Package manager named by the `npm_config_user_agent` env var (set by `bun create`, `npm create`, …). */
export function packageManagerFromUserAgent(ua: string | undefined): PackageManager | undefined {
  const name = (ua ?? "").split("/")[0];
  return PACKAGE_MANAGERS.find((pm) => pm === name);
}

/**
 * The package manager the user most likely wants: the one that launched this
 * CLI, else the first one installed on this machine (bun, pnpm, yarn, npm).
 */
export async function detectPackageManager(): Promise<PackageManager> {
  const fromUa = packageManagerFromUserAgent(process.env.npm_config_user_agent);
  if (fromUa) return fromUa;
  for (const pm of PACKAGE_MANAGERS) {
    if (await commandExists(pm)) return pm;
  }
  return "npm";
}

export type GitResult = "committed" | "initialized" | "skipped-existing-repo" | "unavailable";

/**
 * `git init` + first commit. Skips quietly inside an existing work tree, and
 * keeps the repo even when the commit fails (typically: no user.name/email).
 */
export async function initGit(cwd: string): Promise<GitResult> {
  if (!(await commandExists("git", cwd))) return "unavailable";
  if ((await run("git", ["rev-parse", "--is-inside-work-tree"], cwd)).code === 0) return "skipped-existing-repo";
  if ((await run("git", ["init"], cwd)).code !== 0) return "unavailable";
  await run("git", ["add", "-A"], cwd);
  const commit = await run("git", ["commit", "-m", "Initial commit from create-cratebase"], cwd);
  return commit.code === 0 ? "committed" : "initialized";
}
