import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import { copyTemplate, isNonEmptyDir, projectNameFromDir, renderContents } from "../src/copy-template.js";

describe("renderContents", () => {
  test("replaces every {{PROJECT_NAME}} occurrence", () => {
    const out = renderContents('{ "name": "{{PROJECT_NAME}}" } // {{PROJECT_NAME}}', { projectName: "my-app" });
    expect(out).toBe('{ "name": "my-app" } // my-app');
  });

  test("leaves text with no tokens untouched", () => {
    expect(renderContents("hello world", { projectName: "my-app" })).toBe("hello world");
  });
});

describe("projectNameFromDir", () => {
  test("slugifies the basename", () => {
    expect(projectNameFromDir("./My Cool App")).toBe("my-cool-app");
  });

  test("falls back to a default name for an unusable basename", () => {
    expect(projectNameFromDir("./...")).toBe("cratebase-app");
  });

  test("keeps an already-valid name", () => {
    expect(projectNameFromDir("/tmp/already-valid_name.2")).toBe("already-valid_name.2");
  });
});

describe("copyTemplate + isNonEmptyDir", () => {
  let srcDir: string;
  let dstParent: string;

  beforeEach(async () => {
    srcDir = await mkdtemp(path.join(tmpdir(), "cc-src-"));
    dstParent = await mkdtemp(path.join(tmpdir(), "cc-dst-"));

    await mkdir(path.join(srcDir, "nested"), { recursive: true });
    await writeFile(path.join(srcDir, "package.json"), '{ "name": "{{PROJECT_NAME}}" }');
    await writeFile(path.join(srcDir, "gitignore"), "node_modules/\n.env\n");
    await writeFile(path.join(srcDir, "env.example"), "CRATEBASE_URL=http://localhost:8090\n");
    await writeFile(path.join(srcDir, "nested", "readme.md"), "# {{PROJECT_NAME}}\n");
  });

  afterEach(async () => {
    await rm(srcDir, { recursive: true, force: true });
    await rm(dstParent, { recursive: true, force: true });
  });

  test("renames gitignore -> .gitignore and env.example -> .env.example", async () => {
    const target = path.join(dstParent, "app");
    await copyTemplate(srcDir, target, { projectName: "my-app" });

    expect(await readFile(path.join(target, ".gitignore"), "utf8")).toContain("node_modules/");
    expect(await readFile(path.join(target, ".env.example"), "utf8")).toContain("CRATEBASE_URL");
  });

  test("renders {{PROJECT_NAME}} in text files, including nested ones", async () => {
    const target = path.join(dstParent, "app2");
    await copyTemplate(srcDir, target, { projectName: "my-app" });

    const pkg = await readFile(path.join(target, "package.json"), "utf8");
    expect(pkg).toBe('{ "name": "my-app" }');

    const readme = await readFile(path.join(target, "nested", "readme.md"), "utf8");
    expect(readme).toBe("# my-app\n");
  });

  test("isNonEmptyDir reports false for missing/empty, true for occupied", async () => {
    const missing = path.join(dstParent, "does-not-exist");
    expect(await isNonEmptyDir(missing)).toBe(false);

    const empty = path.join(dstParent, "empty");
    await mkdir(empty);
    expect(await isNonEmptyDir(empty)).toBe(false);

    const occupied = path.join(dstParent, "occupied");
    await mkdir(occupied);
    await writeFile(path.join(occupied, "file.txt"), "x");
    expect(await isNonEmptyDir(occupied)).toBe(true);
  });
});
