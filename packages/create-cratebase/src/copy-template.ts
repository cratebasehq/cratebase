import { mkdir, readdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";

/**
 * Files that get renamed on the way out of `templates/<id>/` so the
 * create-cratebase package's own git tree (and npm publish) never has to deal
 * with a literal `.gitignore`/`.npmignore` inside a nested directory.
 */
const RENAME_ON_COPY: Record<string, string> = {
  gitignore: ".gitignore",
  "env.example": ".env.example",
};

/** Binary-ish extensions copied byte-for-byte, never text-rendered. */
const BINARY_EXTENSIONS = new Set([".png", ".jpg", ".jpeg", ".gif", ".ico", ".woff", ".woff2", ".ttf", ".otf"]);

export interface RenderContext {
  projectName: string;
}

/** Replaces `{{PROJECT_NAME}}` tokens in a text file's contents. */
export function renderContents(contents: string, ctx: RenderContext): string {
  return contents.replaceAll("{{PROJECT_NAME}}", ctx.projectName);
}

function destName(sourceName: string): string {
  return RENAME_ON_COPY[sourceName] ?? sourceName;
}

/**
 * Recursively copies a template directory into a target directory, applying
 * filename renames (see RENAME_ON_COPY) and `{{PROJECT_NAME}}` substitution
 * in text files. Creates `targetDir` if it doesn't exist. Does not touch
 * `node_modules` or `.git` if present in the source (neither ever is, but
 * this keeps the function safe if a template grows one during development).
 */
export async function copyTemplate(sourceDir: string, targetDir: string, ctx: RenderContext): Promise<string[]> {
  const written: string[] = [];
  await copyDir(sourceDir, targetDir, ctx, written);
  return written;
}

async function copyDir(sourceDir: string, targetDir: string, ctx: RenderContext, written: string[]): Promise<void> {
  await mkdir(targetDir, { recursive: true });
  const entries = await readdir(sourceDir, { withFileTypes: true });

  for (const entry of entries) {
    if (entry.name === "node_modules" || entry.name === ".git") continue;

    const sourcePath = path.join(sourceDir, entry.name);
    const targetPath = path.join(targetDir, destName(entry.name));

    if (entry.isDirectory()) {
      await copyDir(sourcePath, targetPath, ctx, written);
      continue;
    }

    if (!entry.isFile()) continue;

    const ext = path.extname(entry.name);
    if (BINARY_EXTENSIONS.has(ext)) {
      const buf = await readFile(sourcePath);
      await writeFile(targetPath, buf);
    } else {
      const text = await readFile(sourcePath, "utf8");
      await writeFile(targetPath, renderContents(text, ctx));
    }
    written.push(targetPath);
  }
}

/** True if `dir` exists and is a non-empty directory. */
export async function isNonEmptyDir(dir: string): Promise<boolean> {
  try {
    const st = await stat(dir);
    if (!st.isDirectory()) return true; // exists as a file — treat as "occupied"
    const entries = await readdir(dir);
    return entries.length > 0;
  } catch {
    return false;
  }
}

/** Derives a valid, lowercase package-name-ish project name from a directory path. */
export function projectNameFromDir(dir: string): string {
  const base = path.basename(path.resolve(dir));
  const slug = base
    .toLowerCase()
    .replace(/[^a-z0-9-_.]+/g, "-")
    .replace(/^[-_.]+|[-_.]+$/g, "");
  return slug.length > 0 ? slug : "cratebase-app";
}
