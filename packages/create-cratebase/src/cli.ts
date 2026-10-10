#!/usr/bin/env node
import * as p from "@clack/prompts";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { ArgParseError, HELP_TEXT, parseArgs, type CliOptions } from "./args.js";
import { copyTemplate, emptyDir, isNonEmptyDir, projectNameFromDir, validateProjectDir } from "./copy-template.js";
import { displayPath, nextSteps } from "./output.js";
import { commandExists, detectPackageManager, initGit, run } from "./post-steps.js";
import {
  PACKAGE_MANAGERS,
  TEMPLATES,
  installCommand,
  type PackageManager,
  type TemplateId,
} from "./templates.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// dist/cli.js -> ../templates ; src/cli.ts (bun run) -> ../templates. Same relative depth either way.
const PACKAGE_ROOT = path.resolve(__dirname, "..");
const TEMPLATES_DIR = path.join(PACKAGE_ROOT, "templates");

const DEFAULT_PROJECT_DIR = "cratebase-app";
const DEFAULT_TEMPLATE: TemplateId = "nextjs";

/** Exit code for a user-cancelled run (Ctrl+C / Esc), matching the shell's SIGINT convention. */
const EXIT_CANCELLED = 130;

function cancelled(): never {
  p.cancel("Cancelled — nothing was written.");
  process.exit(EXIT_CANCELLED);
}

/** Unwraps a clack prompt result, exiting cleanly if the user cancelled. */
function ask<T>(value: T | symbol): T {
  if (p.isCancel(value)) cancelled();
  return value as T;
}

function fail(message: string): never {
  p.cancel(message);
  process.exit(1);
}

async function readVersion(): Promise<string> {
  const pkg = JSON.parse(await readFile(path.join(PACKAGE_ROOT, "package.json"), "utf8")) as { version: string };
  return pkg.version;
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
    console.log(await readVersion());
    return;
  }

  // Prompts need a terminal. Piped/CI runs behave like `--yes` instead of hanging.
  const interactive = !options.yes && Boolean(process.stdin.isTTY && process.stdout.isTTY);

  p.intro(`create-cratebase v${await readVersion()}`);

  // 1. Project directory.
  let projectDir = options.projectDir;
  if (!projectDir) {
    projectDir = interactive
      ? ask(
          await p.text({
            message: "Where should we create your project?",
            placeholder: `./${DEFAULT_PROJECT_DIR}`,
            defaultValue: `./${DEFAULT_PROJECT_DIR}`,
            validate: (value) => (value.trim() === "" ? undefined : validateProjectDir(value)),
          }),
        )
      : `./${DEFAULT_PROJECT_DIR}`;
  }
  projectDir = projectDir.trim();
  const invalid = validateProjectDir(projectDir);
  if (invalid) fail(invalid);

  const targetDir = path.resolve(process.cwd(), projectDir);
  const relDir = displayPath(process.cwd(), targetDir);
  const projectName = projectNameFromDir(targetDir);
  if (path.basename(targetDir).toLowerCase() !== projectName) {
    p.log.info(`Package name will be "${projectName}".`);
  }

  // 2. Target directory conflicts.
  if (await isNonEmptyDir(targetDir)) {
    if (options.force) {
      p.log.warn(`${relDir} isn't empty — clearing it (--force, .git is kept).`);
      await emptyDir(targetDir);
    } else if (interactive) {
      const choice = ask(
        await p.select({
          message: `${relDir === "." ? "The current directory" : relDir} isn't empty. What now?`,
          options: [
            { value: "cancel", label: "Cancel" },
            { value: "merge", label: "Keep existing files", hint: "template files overwrite same-named ones" },
            { value: "empty", label: "Remove existing files and continue", hint: ".git is kept" },
          ],
          initialValue: "cancel",
        }),
      );
      if (choice === "cancel") cancelled();
      if (choice === "empty") await emptyDir(targetDir);
    } else {
      fail(`${targetDir} isn't empty. Pick another directory, or pass --force to clear it.`);
    }
  }

  // 3. Template.
  const template: TemplateId =
    options.template ??
    (interactive
      ? ask(
          await p.select({
            message: "Which template?",
            options: TEMPLATES.map((t) => ({ value: t.id, label: t.title, hint: t.description })),
            initialValue: DEFAULT_TEMPLATE,
          }),
        )
      : DEFAULT_TEMPLATE);

  // 4. Package manager.
  const detectedPm = await detectPackageManager();
  const pm: PackageManager =
    options.pm ??
    (interactive
      ? ask(
          await p.select({
            message: "Which package manager?",
            options: PACKAGE_MANAGERS.map((value) => ({
              value,
              label: value,
              hint: value === detectedPm ? "detected" : undefined,
            })),
            initialValue: detectedPm,
          }),
        )
      : detectedPm);

  // 5. Install deps? Init git? Only ask what a flag didn't already decide.
  let install = options.install;
  let git = options.git;
  if (interactive) {
    install = ask(await p.confirm({ message: `Install dependencies with ${pm}?`, initialValue: install }));
    git = ask(await p.confirm({ message: "Initialize a git repository?", initialValue: git }));
  }

  // 6. Do the work.
  const templateInfo = TEMPLATES.find((t) => t.id === template)!;
  const copySpinner = p.spinner();
  copySpinner.start(`Creating ${templateInfo.title} project in ${relDir}`);
  try {
    await copyTemplate(path.join(TEMPLATES_DIR, template), targetDir, { projectName });
  } catch (err) {
    copySpinner.stop("Couldn't write the project files.", 1);
    fail(err instanceof Error ? err.message : String(err));
  }
  copySpinner.stop(`Created ${templateInfo.title} project in ${relDir}`);

  let installed = false;
  if (install) {
    const { command, args } = installCommand(pm);
    const installSpinner = p.spinner();
    installSpinner.start(`Installing dependencies with ${pm} (this can take a minute)`);
    const result = await run(command, args, targetDir);
    installed = result.code === 0;
    if (installed) {
      installSpinner.stop("Installed dependencies");
    } else {
      installSpinner.stop(`${command} ${args.join(" ")} failed (exit ${result.code})`, 2);
      p.log.warn(`${result.output.trim().split("\n").slice(-12).join("\n")}\n\nRun \`${pm} install\` yourself once that's fixed.`);
    }
  }

  if (git) {
    const gitSpinner = p.spinner();
    gitSpinner.start("Initializing git repository");
    switch (await initGit(targetDir)) {
      case "committed":
        gitSpinner.stop("Initialized git repository with a first commit");
        break;
      case "initialized":
        gitSpinner.stop("Initialized git repository (first commit skipped — set git user.name/user.email)");
        break;
      case "skipped-existing-repo":
        gitSpinner.stop("Already inside a git repository — left git alone");
        break;
      case "unavailable":
        gitSpinner.stop("git isn't available — skipped", 1);
        break;
    }
  }

  const cratebaseInstalled = await commandExists("cratebase");
  p.note(nextSteps({ relDir, pm, template, installed, cratebaseInstalled }), "Next steps");
  p.outro("Happy building!");
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
