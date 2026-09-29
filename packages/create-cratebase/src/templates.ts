export type TemplateId = "nextjs" | "vite-react" | "expo";

export interface TemplateInfo {
  id: TemplateId;
  title: string;
  description: string;
}

export const TEMPLATES: TemplateInfo[] = [
  {
    id: "nextjs",
    title: "Next.js",
    description: "App Router, TypeScript, Tailwind — SSR-safe Cratebase client setup",
  },
  {
    id: "vite-react",
    title: "Vite + React",
    description: "Vite, React, TypeScript, Tailwind — a single-page app",
  },
  {
    id: "expo",
    title: "Expo",
    description: "Expo Router, TypeScript — a lean React Native + web starter",
  },
];

export const TEMPLATE_IDS: readonly TemplateId[] = TEMPLATES.map((t) => t.id);

export function isTemplateId(value: string): value is TemplateId {
  return (TEMPLATE_IDS as readonly string[]).includes(value);
}

export type PackageManager = "bun" | "npm" | "pnpm" | "yarn";

export const PACKAGE_MANAGERS: readonly PackageManager[] = ["bun", "npm", "pnpm", "yarn"];

export function isPackageManager(value: string): value is PackageManager {
  return (PACKAGE_MANAGERS as readonly string[]).includes(value);
}

/** The install command for a given package manager, run inside the target directory. */
export function installCommand(pm: PackageManager): { command: string; args: string[] } {
  switch (pm) {
    case "bun":
      return { command: "bun", args: ["install"] };
    case "npm":
      return { command: "npm", args: ["install"] };
    case "pnpm":
      return { command: "pnpm", args: ["install"] };
    case "yarn":
      return { command: "yarn", args: ["install"] };
  }
}

/** The `run <script>` invocation for a given package manager. */
export function runScriptCommand(pm: PackageManager, script: string): { command: string; args: string[] } {
  switch (pm) {
    case "bun":
      return { command: "bun", args: ["run", script] };
    case "npm":
      return { command: "npm", args: ["run", script] };
    case "pnpm":
      return { command: "pnpm", args: ["run", script] };
    case "yarn":
      return { command: "yarn", args: [script] };
  }
}

export function devCommand(pm: PackageManager): string {
  switch (pm) {
    case "bun":
      return "bun run dev";
    case "npm":
      return "npm run dev";
    case "pnpm":
      return "pnpm dev";
    case "yarn":
      return "yarn dev";
  }
}
