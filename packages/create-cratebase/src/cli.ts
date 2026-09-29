#!/usr/bin/env node
import * as p from "@clack/prompts";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { readFile } from "node:fs/promises";

import { ArgParseError, HELP_TEXT, parseArgs, type CliOptions } from "./args.js";
import { copyTemplate, isNonEmptyDir, projectNameFromDir } from "./copy-template.js";
import { TEMPLATES, installCommand, devCommand, type PackageManager, type TemplateId } from "./templates.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// dist/cli.js -> ../templates ; src/cli.ts (bun run) -> ../templates. Same relative depth either way.
const PACKAGE_ROOT = path.resolve(__dirname, "..");
const TEMPLATES_DIR = path.join(PACKAGE_ROOT, "templates");

function run(command: string, args: string[], cwd: string): Promise<number> {
  return new Promise((resolve) => {
    const child = spawn(command, args, { cwd, stdio: "inherit", shell: process.platform === "win32" });
    child.on("close", (code) => resolve(code ?? 1));
    child.on("error", () => resolve(1));
  });
}

async function detectPackageManager(): Promise<PackageManager> {
  const ua = process.env.npm_config_user_agent ?? "";
  if (ua.startsWith("bun")) return "bun";
  if (ua.startsWith("pnpm")) return "pnpm";
  if (ua.startsWith("yarn")) return "yarn";
  if (ua.startsWith("npm")) return "npm";
  return "bun";
}

async function main(): Promise<void> {
  let options: CliOptions;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (err) {
    if (err instanceof ArgParseError) {
      console.error(`create-cratebase: ${err.message}`);
      console.error(HELP_TEXT);
      process.exitCode = 1;
      return;
    }
    throw err;
  }

  if (options.help) {
    console.log(HELP_TEXT);
    return;
  }

  if (options.version) {
    const pkgRaw = await readFile(path.join(PACKAGE_ROOT, "package.json"), "utf8");
    const pkg = JSON.parse(pkgRaw) as { version: string };
    console.log(pkg.version);
    return;
  }

  p.intro("create-cratebase");

  // 1. Project directory.
  let projectDir = options.projectDir;
  if (!projectDir) {
    if (options.yes) {
      projectDir = "./cratebase-app";
    } else {
      const answer = await p.text({
        message: "Where should we create your project?",
        placeholder: "./my-app",
        validate: (value) => (value.trim().length === 0 ? "Please enter a directory name." : undefined),
      });
      if (p.isCancel(answer)) {
        p.cancel("Cancelled.");
        process.exit(1);
      }
      projectDir = answer;
    }
  }

  const targetDir = path.resolve(process.cwd(), projectDir);
  if (await isNonEmptyDir(targetDir)) {
    p.cancel(`Refusing to scaffold into non-empty directory: ${targetDir}`);
    process.exit(1);
  }

  // 2. Template.
  let template: TemplateId | undefined = options.template;
  if (!template) {
    if (options.yes) {
      template = "nextjs";
    } else {
      const answer = await p.select({
        message: "Which template?",
        options: TEMPLATES.map((t) => ({ value: t.id, label: t.title, hint: t.description })),
      });
      if (p.isCancel(answer)) {
        p.cancel("Cancelled.");
        process.exit(1);
      }
      template = answer;
    }
  }

  // 3. Package manager.
  let pm: PackageManager | undefined = options.pm;
  if (!pm) {
    if (options.yes) {
      pm = await detectPackageManager();
    } else {
      const detected = await detectPackageManager();
      const answer = await p.select({
        message: "Which package manager?",
        options: [
          { value: "bun", label: "bun" },
          { value: "npm", label: "npm" },
          { value: "pnpm", label: "pnpm" },
          { value: "yarn", label: "yarn" },
        ],
        initialValue: detected,
      });
      if (p.isCancel(answer)) {
        p.cancel("Cancelled.");
        process.exit(1);
      }
      pm = answer as PackageManager;
    }
  }

  // 4. Install deps? Init git?
  let install = options.install;
  let git = options.git;
  if (!options.yes) {
    const installAnswer = await p.confirm({ message: "Install dependencies?", initialValue: options.install });
    if (p.isCancel(installAnswer)) {
      p.cancel("Cancelled.");
      process.exit(1);
    }
    install = installAnswer;

    const gitAnswer = await p.confirm({ message: "Initialize a git repository?", initialValue: options.git });
    if (p.isCancel(gitAnswer)) {
      p.cancel("Cancelled.");
      process.exit(1);
    }
    git = gitAnswer;
  }

  const projectName = projectNameFromDir(targetDir);
  const templateDir = path.join(TEMPLATES_DIR, template);

  const copySpinner = p.spinner();
  copySpinner.start(`Scaffolding ${template} into ${path.relative(process.cwd(), targetDir) || "."}`);
  await copyTemplate(templateDir, targetDir, { projectName });
  copySpinner.stop(`Scaffolded ${TEMPLATES.find((t) => t.id === template)?.title} into ${targetDir}`);

  if (install) {
    const { command, args } = installCommand(pm);
    p.log.step(`Installing dependencies with ${pm}…`);
    const code = await run(command, args, targetDir);
    if (code !== 0) {
      p.log.warn(`${command} ${args.join(" ")} exited with code ${code}. You can re-run it yourself later.`);
    }
  }

  if (git) {
    p.log.step("Initializing git repository…");
    await run("git", ["init"], targetDir);
    await run("git", ["add", "-A"], targetDir);
    await run("git", ["commit", "-m", "Initial commit from create-cratebase"], targetDir);
  }

  const relDir = path.relative(process.cwd(), targetDir) || ".";
  const steps = [`cd ${relDir}`];
  if (!install) steps.push(`${pm} install`);
  steps.push(devCommand(pm));

  p.outro(
    [
      "Next steps:",
      "",
      ...steps.map((s) => `  ${s}`),
      "",
      "`dev` starts the Cratebase server (installing the binary on first run if needed), watches types, and runs the frontend.",
      "Docs: https://cratebase.dev/docs/getting-started/starter-kits/",
    ].join("\n"),
  );
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
