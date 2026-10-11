import { describe, expect, test } from "bun:test";
import {
  collectionId,
  defineCollection,
  defineSchema,
  idx,
  relation,
  select,
  text,
  number,
} from "../src/index.ts";

describe("collectionId", () => {
  test("matches the server's crc32 vectors", () => {
    expect(collectionId("base", "posts")).toBe("pbc_1125843985");
    expect(collectionId("base", "_mfas")).toBe("pbc_2279338944");
    expect(collectionId("auth", "_superusers")).toBe("pbc_3142635823");
  });
});

describe("idx", () => {
  test("builds backtick-quoted statements", () => {
    expect(idx("idx_a", "posts", ["slug"], { unique: true })).toBe(
      "CREATE UNIQUE INDEX `idx_a` ON `posts` (`slug`)",
    );
    expect(idx("idx_b", "posts", ["a", "b"], { where: "a != ''" })).toBe(
      "CREATE INDEX `idx_b` ON `posts` (`a`, `b`) WHERE a != ''",
    );
  });
});

describe("defineSchema", () => {
  const authors = defineCollection({
    name: "authors",
    fields: [text("name", { required: true, max: 100 })],
  });
  const posts = defineCollection({
    name: "posts",
    fields: [
      text("title", { required: true, searchable: true }),
      select("status", ["draft", "live"], { maxSelect: 1 }),
      number("views", { onlyInt: true, min: 0 }),
      relation("author", "authors", { cascadeDelete: true }),
      relation("owner", "users"),
    ],
    indexes: [idx("idx_posts_title", "posts", ["title"], { unique: true })],
    listRule: "",
  });

  test("resolves relations and emits the expected document", () => {
    const doc = JSON.parse(JSON.stringify(defineSchema(authors, posts)));
    expect(doc).toEqual({
      collections: [
        {
          id: collectionId("base", "authors"),
          name: "authors",
          type: "base",
          fields: [{ name: "name", type: "text", required: true, max: 100 }],
          indexes: [],
          listRule: null,
          viewRule: null,
          createRule: null,
          updateRule: null,
          deleteRule: null,
        },
        {
          id: collectionId("base", "posts"),
          name: "posts",
          type: "base",
          fields: [
            { name: "title", type: "text", required: true, searchable: true },
            { name: "status", type: "select", maxSelect: 1, values: ["draft", "live"] },
            { name: "views", type: "number", onlyInt: true, min: 0 },
            {
              name: "author",
              type: "relation",
              cascadeDelete: true,
              collectionId: collectionId("base", "authors"),
            },
            { name: "owner", type: "relation", collectionId: "_pb_users_auth_" },
          ],
          indexes: ["CREATE UNIQUE INDEX `idx_posts_title` ON `posts` (`title`)"],
          listRule: "",
          viewRule: null,
          createRule: null,
          updateRule: null,
          deleteRule: null,
        },
      ],
    });
  });

  test("does not mutate input fields", () => {
    defineSchema(authors, posts);
    expect(posts.fields[3]).not.toHaveProperty("collectionId");
  });

  test("unknown relation target throws", () => {
    expect(() => defineSchema(posts)).toThrow(
      'relation "author": collection "authors" is not defined in this schema (built-ins: users, _superusers)',
    );
  });

  test("duplicate names throw", () => {
    expect(() => defineSchema(authors, authors)).toThrow(/duplicate collection "authors"/);
  });
});
