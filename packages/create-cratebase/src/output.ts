import path from "node:path";

import { devCommand, devUrl, scriptCommandLine, type PackageManager, type TemplateId } from "./templates.js";

export const DOCS_URL = "https://cratebase.dev/docs/getting-started/starter-kits/";
export const INSTALL_CRATEBASE = "curl -fsSL https://cratebase.dev/install.sh | sh";

export interface NextStepsInput {
  relDir: string;
  pm: PackageManager;
  template: TemplateId;
  installed: boolean;
  cratebaseInstalled: boolean;
}

/** The body of the closing "Next steps" box. Pure, so it can be unit-tested. */
export function nextSteps(input: NextStepsInput): string {
  const { relDir, pm, template, installed, cratebaseInstalled } = input;
  const lines: string[] = [];
  let n = 1;
  const step = (text: string) => lines.push(`${n++}. ${text}`);

  if (relDir !== ".") step(`cd ${quoteIfNeeded(relDir)}`);
  if (!cratebaseInstalled) step(`${INSTALL_CRATEBASE}   # the Cratebase server binary`);
  if (!installed) step(`${pm} install`);
  step(devCommand(pm));

  lines.push(
    "",
    `App:        ${devUrl(template)}`,
    "Cratebase:  http://localhost:8090/_/   (admin dashboard)",
    "Demo login: demo@example.com / password123",
    "",
    `Types:      ${scriptCommandLine(pm, "typegen")}   (regenerate after schema changes)`,
    `Docs:       ${DOCS_URL}`,
  );
  return lines.join("\n");
}

function quoteIfNeeded(p: string): string {
  return /\s/.test(p) ? JSON.stringify(p) : p;
}

/** `target` relative to `cwd` when it's inside it, else the absolute path (never `../../..`). */
export function displayPath(cwd: string, target: string): string {
  const rel = path.relative(cwd, target) || ".";
  return rel.startsWith("..") || path.isAbsolute(rel) ? target : rel;
}
