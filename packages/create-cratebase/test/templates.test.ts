import { describe, expect, test } from "bun:test";
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";

import { TEMPLATES, TEMPLATE_IDS, isTemplateId } from "../src/templates.js";

const TEMPLATES_DIR = path.resolve(import.meta.dir, "..", "templates");

async function* walk(dir: string): AsyncGenerator<string> {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (entry.name === "node_modules") continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(full);
    else yield full;
  }
}

describe("templates", () => {
  test("registry lists every template directory and nothing else", async () => {
    const dirs = (await readdir(TEMPLATES_DIR, { withFileTypes: true })).filter((d) => d.isDirectory()).map((d) => d.name);
    expect([...dirs].sort()).toEqual([...TEMPLATE_IDS].sort());
    expect(isTemplateId("tanstack-start")).toBe(true);
    for (const t of TEMPLATES) expect(t.description.length).toBeGreaterThan(10);
  });

  for (const id of TEMPLATE_IDS) {
    describe(id, () => {
      test("ships the standard Cratebase files", async () => {
        for (const file of [
          "package.json",
          "README.md",
          "schema.json",
          "gitignore",
          "env.example",
          "cratebase-types.d.ts",
          "scripts/cratebase-dev.mjs",
          "cb_hooks/welcome.cb.js",
          "cb_seed/01-users.json",
          "cb_seed/02-posts.json",
        ]) {
          expect((await stat(path.join(TEMPLATES_DIR, id, file))).isFile()).toBe(true);
        }
      });

      test("package.json has a dev script that starts Cratebase", async () => {
        const pkg = JSON.parse(await readFile(path.join(TEMPLATES_DIR, id, "package.json"), "utf8")) as {
          name: string;
          scripts: Record<string, string>;
        };
        expect(pkg.name).toBe("{{PROJECT_NAME}}");
        expect(pkg.scripts.dev).toContain("cratebase-dev.mjs");
      });

      test("uses the Cratebase naming (no legacy names)", async () => {
        for await (const file of walk(path.join(TEMPLATES_DIR, id))) {
          const text = await readFile(file, "utf8").catch(() => "");
          // `_pb_users_auth_` is the built-in users collection id, not a legacy name.
          const cleaned = text.replaceAll("_pb_users_auth_", "");
          expect(cleaned, file).not.toMatch(/pocketbase|pb_(data|hooks|seed|migrations|public)|\.pb\.js/i);
        }
      });
    });
  }
});
