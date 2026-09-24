import { describe, expect, test } from "bun:test";
import { generate, generateInterface, generateSchema } from "./codegen.js";

/**
 * `@cratebase/client`'s `AnySchema`/`CollectionService` generic bounds
 * are `Record<string, ...>`-shaped. TypeScript only lets a type satisfy
 * an index-signature constraint like that through the "fresh object
 * literal" structural check — a plain `interface` (even one with no
 * properties of its own) never qualifies, so `createClient<Schema>()`
 * would fail to compile against an `interface`-based `Schema`. Every
 * generated shape must therefore be `export type X = {...}`, never
 * `export interface`.
 */
describe("generated output never uses `interface`, only `type` aliases", () => {
  test("generateInterface emits a type alias", () => {
    const out = generateInterface({
      name: "posts",
      type: "base",
      fields: [{ name: "title", type: "text", required: true }],
    });
    expect(out).toContain("export type PostsRecord = {");
    expect(out).not.toContain("interface");
  });

  test("generateSchema emits a type alias", () => {
    const out = generateSchema([{ name: "posts", fields: [] }]);
    expect(out).toContain("export type Schema = {");
    expect(out).not.toContain("interface");
  });

  test("generate() end to end never contains the word interface", () => {
    const out = generate([
      {
        name: "posts",
        type: "base",
        fields: [
          { name: "title", type: "text", required: true },
          { name: "tags", type: "select", maxSelect: 5 },
          { name: "author", type: "relation", maxSelect: 1 },
        ],
      },
    ]);
    expect(out).not.toContain("interface");
    expect(out).toContain("export type PostsRecord = {");
    expect(out).toContain("export type Schema = {");
  });
});
