export type TemplateId = "nextjs" | "vite-react" | "expo" | "tanstack-start";

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
    id: "tanstack-start",
    title: "TanStack Start",
    description: "TanStack Start on Cloudflare Workers — SSR cookie sessions via @cratebase/start",
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

/** A script invocation as the user would type it, e.g. `bun run dev` / `pnpm dev`. */
export function scriptCommandLine(pm: PackageManager, script: string): string {
  return pm === "pnpm" || pm === "yarn" ? `${pm} ${script}` : `${pm} run ${script}`;
}

export function devCommand(pm: PackageManager): string {
  return scriptCommandLine(pm, "dev");
}

/** Where the frontend's dev server listens, for the "next steps" output. */
export function devUrl(template: TemplateId): string {
  switch (template) {
    case "vite-react":
      return "http://localhost:5173";
    case "expo":
      return "http://localhost:8081";
    case "nextjs":
    case "tanstack-start":
      return "http://localhost:3000";
  }
}
