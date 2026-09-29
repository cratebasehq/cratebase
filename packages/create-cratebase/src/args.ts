import { isPackageManager, isTemplateId, type PackageManager, type TemplateId } from "./templates.js";

export interface CliOptions {
  /** Positional argument: the directory to scaffold into (may be relative or "."). */
  projectDir: string | undefined;
  template: TemplateId | undefined;
  pm: PackageManager | undefined;
  /** Skip every interactive prompt and use defaults for anything not passed as a flag. */
  yes: boolean;
  /** Run the package manager's install step after copying the template. Default true. */
  install: boolean;
  /** Run `git init` (+ initial commit) in the new project. Default true. */
  git: boolean;
  help: boolean;
  version: boolean;
}

export class ArgParseError extends Error {}

/**
 * Parses argv (excluding `node`/script path) into structured options.
 * Unknown flags raise ArgParseError; unknown values for --template/--pm do too.
 */
export function parseArgs(argv: string[]): CliOptions {
  const options: CliOptions = {
    projectDir: undefined,
    template: undefined,
    pm: undefined,
    yes: false,
    install: true,
    git: true,
    help: false,
    version: false,
  };

  const positionals: string[] = [];

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) continue;

    if (arg === "--template" || arg === "-t") {
      const value = argv[++i];
      if (!value || !isTemplateId(value)) {
        throw new ArgParseError(
          `Invalid --template value: ${value ?? "(missing)"}. Expected one of: nextjs, vite-react, expo.`,
        );
      }
      options.template = value;
      continue;
    }

    if (arg === "--pm") {
      const value = argv[++i];
      if (!value || !isPackageManager(value)) {
        throw new ArgParseError(`Invalid --pm value: ${value ?? "(missing)"}. Expected one of: bun, npm, pnpm, yarn.`);
      }
      options.pm = value;
      continue;
    }

    if (arg.startsWith("--template=")) {
      const value = arg.slice("--template=".length);
      if (!isTemplateId(value)) {
        throw new ArgParseError(`Invalid --template value: ${value}. Expected one of: nextjs, vite-react, expo.`);
      }
      options.template = value;
      continue;
    }

    if (arg.startsWith("--pm=")) {
      const value = arg.slice("--pm=".length);
      if (!isPackageManager(value)) {
        throw new ArgParseError(`Invalid --pm value: ${value}. Expected one of: bun, npm, pnpm, yarn.`);
      }
      options.pm = value;
      continue;
    }

    if (arg === "--yes" || arg === "-y") {
      options.yes = true;
      continue;
    }
    if (arg === "--install") {
      options.install = true;
      continue;
    }
    if (arg === "--no-install") {
      options.install = false;
      continue;
    }
    if (arg === "--git") {
      options.git = true;
      continue;
    }
    if (arg === "--no-git") {
      options.git = false;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    if (arg === "--version" || arg === "-v") {
      options.version = true;
      continue;
    }

    if (arg.startsWith("-")) {
      throw new ArgParseError(`Unknown flag: ${arg}`);
    }

    positionals.push(arg);
  }

  if (positionals.length > 1) {
    throw new ArgParseError(`Expected a single project-directory argument, got: ${positionals.join(", ")}`);
  }
  options.projectDir = positionals[0];

  return options;
}

export const HELP_TEXT = `
create-cratebase — scaffold a new app on Cratebase

Usage:
  bun create cratebase <project-directory> [options]
  npm create cratebase@latest <project-directory> [options]

Options:
  -t, --template <name>   Template to use: nextjs, vite-react, expo
      --pm <name>         Package manager: bun, npm, pnpm, yarn
  -y, --yes               Skip prompts, accept defaults for anything unset
      --no-install        Skip installing dependencies
      --no-git            Skip \`git init\`
  -h, --help               Print this help
  -v, --version            Print the CLI version

Examples:
  bun create cratebase my-app
  bun create cratebase my-app --template nextjs --pm bun --yes
  npm create cratebase@latest my-app -- --template vite-react --no-install
`;
