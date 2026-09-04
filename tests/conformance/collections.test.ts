import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { adminClient, dropCollection, expectError, uniq } from "./harness";

/**
 * Collections API: /api/collections (superuser only).
 * Everything here uses the v0.23+ `fields` shape (not the old `schema`).
 */
let pb: PocketBase;
const created: string[] = [];

beforeAll(async () => {
  pb = await adminClient();
});

afterAll(async () => {
  for (const name of created.reverse()) await dropCollection(pb, name);
});

function track(name: string) {
  created.push(name);
  return name;
}

describe("collections: create", () => {
  test("base collection with every field type and its options", async () => {
    const relTarget = track(uniq("rel_target"));
    const target = await pb.collections.create({ name: relTarget, type: "base", fields: [] });
    expect(target.id).toMatch(/^pbc_\d+$/);

    const name = track(uniq("all_fields"));
    const col = await pb.collections.create({
      name,
      type: "base",
      fields: [
        { name: "title", type: "text", required: true, min: 2, max: 50, pattern: "^[a-zA-Z0-9 ]+$" },
        { name: "slug", type: "text", autogeneratePattern: "[a-z]{8}" },
        { name: "body", type: "editor", maxSize: 100000, convertURLs: true },
        { name: "count", type: "number", min: 0, max: 100, onlyInt: true },
        { name: "active", type: "bool" },
        { name: "contact", type: "email", exceptDomains: ["spam.test"], onlyDomains: [] },
        { name: "site", type: "url", onlyDomains: ["example.com"] },
        { name: "when", type: "date", min: "2020-01-01 00:00:00.000Z", max: "2030-01-01 00:00:00.000Z" },
        { name: "kind", type: "select", values: ["a", "b", "c"], maxSelect: 1 },
        { name: "tags", type: "select", values: ["x", "y", "z"], maxSelect: 3 },
        { name: "doc", type: "file", maxSelect: 1, maxSize: 1024, mimeTypes: ["text/plain"], thumbs: ["100x100"], protected: false },
        { name: "owner", type: "relation", collectionId: target.id, cascadeDelete: false, minSelect: 0, maxSelect: 1 },
        { name: "meta", type: "json", maxSize: 2000 },
        { name: "pos", type: "geoPoint" },
        { name: "created", type: "autodate", onCreate: true, onUpdate: false },
        { name: "updated", type: "autodate", onCreate: true, onUpdate: true },
      ],
    });

    expect(col.name).toBe(name);
    expect(col.type).toBe("base");
    expect(col.system).toBe(false);
    expect(col.listRule).toBeNull();
    expect(col.viewRule).toBeNull();
    expect(col.createRule).toBeNull();
    expect(col.updateRule).toBeNull();
    expect(col.deleteRule).toBeNull();
    expect(col.indexes).toEqual([]);
    expect(col.created).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}Z$/);

    const byName = Object.fromEntries(col.fields.map((f: any) => [f.name, f]));
    // The `id` primary key is injected automatically.
    expect(byName.id).toMatchObject({
      type: "text",
      primaryKey: true,
      system: true,
      required: true,
      min: 15,
      max: 15,
      pattern: "^[a-z0-9]+$",
      autogeneratePattern: "[a-z0-9]{15}",
    });
    expect(byName.id.id).toBe("text3208210256");
    expect(col.fields[0].name).toBe("id");

    expect(byName.title).toMatchObject({ type: "text", required: true, min: 2, max: 50, pattern: "^[a-zA-Z0-9 ]+$", primaryKey: false, hidden: false, presentable: false, system: false });
    expect(byName.title.id).toMatch(/^text\d+$/);
    expect(byName.body).toMatchObject({ type: "editor", maxSize: 100000, convertURLs: true });
    expect(byName.count).toMatchObject({ type: "number", min: 0, max: 100, onlyInt: true });
    expect(byName.active).toMatchObject({ type: "bool" });
    expect(byName.contact).toMatchObject({ type: "email", exceptDomains: ["spam.test"] });
    expect(byName.site).toMatchObject({ type: "url", onlyDomains: ["example.com"] });
    expect(byName.when).toMatchObject({ type: "date", min: "2020-01-01 00:00:00.000Z", max: "2030-01-01 00:00:00.000Z" });
    expect(byName.kind).toMatchObject({ type: "select", values: ["a", "b", "c"], maxSelect: 1 });
    expect(byName.tags).toMatchObject({ type: "select", maxSelect: 3 });
    expect(byName.doc).toMatchObject({ type: "file", maxSelect: 1, maxSize: 1024, mimeTypes: ["text/plain"], thumbs: ["100x100"], protected: false });
    expect(byName.owner).toMatchObject({ type: "relation", collectionId: target.id, cascadeDelete: false, minSelect: 0, maxSelect: 1 });
    expect(byName.meta).toMatchObject({ type: "json", maxSize: 2000 });
    expect(byName.pos).toMatchObject({ type: "geoPoint" });
    expect(byName.created).toMatchObject({ type: "autodate", onCreate: true, onUpdate: false });
    expect(byName.updated).toMatchObject({ type: "autodate", onCreate: true, onUpdate: true });
    expect(byName.created.id).toBe("autodate2990389176");
    expect(byName.updated.id).toBe("autodate3332085495");
  });

  test("auth collection gets the auth system fields and default options", async () => {
    const name = track(uniq("auth_users"));
    const col = await pb.collections.create({ name, type: "auth", fields: [] });
    const names = col.fields.map((f: any) => f.name);
    expect(names).toEqual(expect.arrayContaining(["id", "password", "tokenKey", "email", "emailVisibility", "verified"]));
    const byName = Object.fromEntries(col.fields.map((f: any) => [f.name, f]));
    expect(byName.password).toMatchObject({ type: "password", hidden: true, system: true, required: true });
    expect(byName.tokenKey).toMatchObject({ type: "text", hidden: true, system: true, required: true });
    expect(byName.email).toMatchObject({ type: "email", system: true });
    expect(byName.emailVisibility).toMatchObject({ type: "bool", system: true });
    expect(byName.verified).toMatchObject({ type: "bool", system: true });

    expect(col.passwordAuth).toEqual({ enabled: true, identityFields: ["email"] });
    expect(col.oauth2).toMatchObject({ enabled: false, providers: [] });
    expect(col.mfa).toMatchObject({ enabled: false, duration: 600, rule: "" });
    expect(col.otp).toMatchObject({ enabled: false, duration: 180, length: 8 });
    expect(col.otp.emailTemplate).toHaveProperty("subject");
    expect(col.otp.emailTemplate).toHaveProperty("body");
    expect(col.authRule).toBe("");
    expect(col.manageRule).toBeNull();
    // NOTE: new auth collections default to 5 days (432000s), unlike the
    // built-in `users` collection which ships with 7 days.
    expect(col.authToken).toEqual({ duration: 432000 });
    expect(col.fileToken).toMatchObject({ duration: 180 });
    expect(col.verificationToken).toEqual({ duration: 86400 });
    expect(col.passwordResetToken).toEqual({ duration: 1800 });
    expect(col.emailChangeToken).toEqual({ duration: 1800 });
    expect(col.authAlert).toMatchObject({ enabled: true });
    expect(col.verificationTemplate).toHaveProperty("subject");
    expect(col.resetPasswordTemplate).toHaveProperty("body");
    expect(col.confirmEmailChangeTemplate).toHaveProperty("body");
    // token secrets are never serialized, even for superusers
    expect(col.authToken).not.toHaveProperty("secret");
    expect(col.fileToken).not.toHaveProperty("secret");

    // default indexes: unique email + tokenKey
    expect(col.indexes.length).toBe(2);
    expect(col.indexes.some((i: string) => /UNIQUE INDEX .* \(`?tokenKey`?\)/.test(i))).toBe(true);
    expect(col.indexes.some((i: string) => /UNIQUE INDEX .*email/.test(i))).toBe(true);
  });

  test("view collection derives fields from viewQuery", async () => {
    const base = track(uniq("view_base"));
    const b = await pb.collections.create({
      name: base,
      type: "base",
      fields: [
        { name: "title", type: "text" },
        { name: "n", type: "number" },
      ],
    });
    await pb.collection(base).create({ title: "one", n: 1 });
    await pb.collection(base).create({ title: "two", n: 2 });

    const name = track(uniq("view_col"));
    const v = await pb.collections.create({
      name,
      type: "view",
      viewQuery: `SELECT id, title, n FROM ${base}`,
    });
    expect(v.type).toBe("view");
    expect(v.viewQuery).toBe(`SELECT id, title, n FROM ${base}`);
    const names = v.fields.map((f: any) => f.name);
    expect(names).toEqual(["id", "title", "n"]);
    const byName = Object.fromEntries(v.fields.map((f: any) => [f.name, f]));
    expect(byName.title.type).toBe("text");
    expect(byName.n.type).toBe("number");
    // view fields get synthetic `_clone_*` ids rather than the origin column ids
    expect(byName.title.id).toMatch(/^_clone_/);
    expect(b.fields.find((f: any) => f.name === "title")?.id).toMatch(/^text\d+$/);

    const list = await pb.collection(name).getFullList({ sort: "n" });
    expect(list.map((r) => r.title)).toEqual(["one", "two"]);

    // views are read-only
    const err = await expectError(() => pb.collection(name).create({ title: "x" }));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Unsupported collection type.");
  });

  test("view without id column is rejected", async () => {
    const base = track(uniq("view_noid"));
    await pb.collections.create({ name: base, type: "base", fields: [{ name: "title", type: "text" }] });
    const err = await expectError(() =>
      pb.collections.create({ name: uniq("view_bad"), type: "view", viewQuery: `SELECT title FROM ${base}` }),
    );
    expect(err.status).toBe(400);
    expect(err.response.data.viewQuery.code).toBe("validation_invalid_view_query");
  });

  test("duplicate name -> validation_collection_name_exists", async () => {
    const name = track(uniq("dup"));
    await pb.collections.create({ name, type: "base", fields: [] });
    const err = await expectError(() => pb.collections.create({ name, type: "base", fields: [] }));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to create collection.");
    expect(err.response.data.name.code).toBe("validation_collection_name_exists");
    expect(err.response.data.name.message).toBe("Collection name must be unique (case insensitive).");
  });

  test("invalid name characters", async () => {
    const err = await expectError(() => pb.collections.create({ name: "bad name!", type: "base", fields: [] }));
    expect(err.status).toBe(400);
    expect(err.response.data.name.code).toBe("validation_match_invalid");
  });

  test("invalid API rule -> validation_invalid_rule", async () => {
    const err = await expectError(() =>
      pb.collections.create({ name: uniq("badrule"), type: "base", fields: [], listRule: "nonexistent_field = 1" }),
    );
    expect(err.status).toBe(400);
    expect(err.response.data.listRule.code).toBe("validation_invalid_rule");
  });

  test("invalid index -> validation_invalid_index_expression", async () => {
    const err = await expectError(() =>
      pb.collections.create({ name: uniq("badidx"), type: "base", fields: [], indexes: ["not an index"] }),
    );
    expect(err.status).toBe(400);
    // errors on list-typed props are keyed by index
    expect(err.response.data.indexes["0"].code).toBe("validation_invalid_index_expression");
    expect(err.response.data.indexes["0"].message).toBe("Invalid CREATE INDEX expression.");
  });

  test("unknown / missing field type is a body-format error", async () => {
    const err = await expectError(() =>
      pb.collections.create({ name: uniq("notype"), type: "base", fields: [{ name: "a", type: "bogus" }] }),
    );
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to load the submitted data due to invalid formatting.");
    expect(err.response.data).toEqual({});
  });

  test("duplicate field names collapse silently (last definition wins)", async () => {
    const col = await pb.collections.create({
      name: track(uniq("dupfield")),
      type: "base",
      fields: [
        { name: "a", type: "text" },
        { name: "a", type: "number" },
      ],
    });
    expect(col.fields.map((f: any) => [f.name, f.type])).toEqual([
      ["id", "text"],
      ["a", "number"],
    ]);
  });

  test("system collections are listed and cannot be deleted", async () => {
    const all = await pb.collections.getFullList();
    const sys = all.filter((c) => c.system).map((c) => c.name).sort();
    expect(sys).toEqual(["_authOrigins", "_externalAuths", "_mfas", "_otps", "_superusers"]);
    const users = all.find((c) => c.name === "users");
    expect(users?.id).toBe("_pb_users_auth_");
    expect(users?.type).toBe("auth");

    const err = await expectError(() => pb.collections.delete("_superusers"));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to delete collection.");
  });
});

describe("collections: read / update / delete", () => {
  test("getList pagination envelope + filter + sort", async () => {
    const a = track(uniq("lst_a"));
    const b = track(uniq("lst_b"));
    await pb.collections.create({ name: a, type: "base", fields: [] });
    await pb.collections.create({ name: b, type: "base", fields: [] });
    const page = await pb.collections.getList(1, 1, { filter: `name = "${a}" || name = "${b}"`, sort: "-name" });
    expect(page.page).toBe(1);
    expect(page.perPage).toBe(1);
    expect(page.totalItems).toBe(2);
    expect(page.totalPages).toBe(2);
    expect(page.items[0].name).toBe(b);
  });

  test("getOne by id and by name", async () => {
    const name = track(uniq("getone"));
    const c = await pb.collections.create({ name, type: "base", fields: [] });
    const byId = await pb.collections.getOne(c.id);
    const byName = await pb.collections.getOne(name);
    expect(byId.id).toBe(c.id);
    expect(byName.id).toBe(c.id);
    const err = await expectError(() => pb.collections.getOne("nope_" + name));
    expect(err.status).toBe(404);
    expect(err.response.message).toBe("The requested resource wasn't found.");
    expect(err.response.data).toEqual({});
  });

  test("update: rename field by id keeps data, add field, change rules", async () => {
    const name = track(uniq("upd"));
    const c = await pb.collections.create({ name, type: "base", fields: [{ name: "title", type: "text" }] });
    const rec = await pb.collection(name).create({ title: "hello" });
    const titleField = c.fields.find((f: any) => f.name === "title")!;

    const updated = await pb.collections.update(c.id, {
      fields: [
        ...c.fields.filter((f: any) => f.name !== "title"),
        { ...titleField, name: "heading" },
        { name: "extra", type: "number" },
      ],
      listRule: "",
      viewRule: "",
    });
    expect(updated.fields.map((f: any) => f.name)).toEqual(["id", "heading", "extra"]);
    expect(updated.fields.find((f: any) => f.name === "heading")?.id).toBe(titleField.id);
    expect(updated.listRule).toBe("");
    expect(updated.viewRule).toBe("");

    const again = await pb.collection(name).getOne(rec.id);
    expect(again.heading).toBe("hello");
    expect(again.title).toBeUndefined();
    expect(again.extra).toBe(0);
  });

  test("update: removing a field drops the column", async () => {
    const name = track(uniq("dropf"));
    const c = await pb.collections.create({
      name,
      type: "base",
      fields: [
        { name: "a", type: "text" },
        { name: "b", type: "text" },
      ],
    });
    const rec = await pb.collection(name).create({ a: "1", b: "2" });
    await pb.collections.update(c.id, { fields: c.fields.filter((f: any) => f.name !== "b") });
    const again = await pb.collection(name).getOne(rec.id);
    expect(again.a).toBe("1");
    expect(again).not.toHaveProperty("b");
  });

  test("indexes: create, replace, drop", async () => {
    const name = track(uniq("idx"));
    const c = await pb.collections.create({ name, type: "base", fields: [{ name: "email", type: "text" }] });
    const idxName = `idx_${name}_email`;
    const withIdx = await pb.collections.update(c.id, {
      indexes: [`CREATE UNIQUE INDEX \`${idxName}\` ON \`${name}\` (\`email\`)`],
    });
    expect(withIdx.indexes).toHaveLength(1);
    expect(withIdx.indexes[0]).toContain("UNIQUE INDEX");
    expect(withIdx.indexes[0]).toContain(idxName);

    // the unique index is actually enforced
    await pb.collection(name).create({ email: "a@b.c" });
    const err = await expectError(() => pb.collection(name).create({ email: "a@b.c" }));
    expect(err.status).toBe(400);
    expect(err.response.data.email.code).toBe("validation_not_unique");
    expect(err.response.data.email.message).toBe("Value must be unique.");

    const dropped = await pb.collections.update(c.id, { indexes: [] });
    expect(dropped.indexes).toEqual([]);
    // no longer unique
    await pb.collection(name).create({ email: "a@b.c" });
  });

  test("index referencing unknown column is rejected", async () => {
    const name = track(uniq("idxbad"));
    const c = await pb.collections.create({ name, type: "base", fields: [] });
    const err = await expectError(() =>
      pb.collections.update(c.id, { indexes: [`CREATE INDEX \`idx_${name}\` ON \`${name}\` (\`missing\`)`] }),
    );
    expect(err.status).toBe(400);
    expect(err.response.data.indexes["0"].code).toBe("validation_invalid_index_expression");
    expect(err.response.data.indexes["0"].message).toMatch(/^Failed to create index .* no such column: missing/);
  });

  test("changing the type of an existing collection is rejected", async () => {
    const name = track(uniq("typechg"));
    const c = await pb.collections.create({ name, type: "base", fields: [] });
    const err = await expectError(() => pb.collections.update(c.id, { type: "auth" }));
    expect(err.status).toBe(400);
    expect(err.response.data.type.code).toBe("validation_collection_type_change");
  });

  test("truncate removes all records", async () => {
    const name = track(uniq("trunc"));
    await pb.collections.create({ name, type: "base", fields: [{ name: "t", type: "text" }] });
    await pb.collection(name).create({ t: "1" });
    await pb.collection(name).create({ t: "2" });
    expect((await pb.collection(name).getList(1, 1)).totalItems).toBe(2);
    const res = await pb.collections.truncate(name);
    expect(res).toBe(true);
    expect((await pb.collection(name).getList(1, 1)).totalItems).toBe(0);
  });

  test("delete removes collection and its records; 404 afterwards", async () => {
    const name = uniq("del");
    const c = await pb.collections.create({ name, type: "base", fields: [] });
    await pb.collection(name).create({});
    expect(await pb.collections.delete(c.id)).toBe(true);
    const err = await expectError(() => pb.collections.getOne(c.id));
    expect(err.status).toBe(404);
    const err2 = await expectError(() => pb.collection(name).getList());
    expect(err2.status).toBe(404);
    expect(err2.response.message).toBe("Missing collection context.");
  });

  test("delete of a collection referenced by a relation is rejected", async () => {
    const target = track(uniq("reftarget"));
    const t = await pb.collections.create({ name: target, type: "base", fields: [] });
    const src = track(uniq("refsrc"));
    await pb.collections.create({
      name: src,
      type: "base",
      fields: [{ name: "rel", type: "relation", collectionId: t.id, maxSelect: 1 }],
    });
    const err = await expectError(() => pb.collections.delete(t.id));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe(`Failed to delete collection probably due to existing reference in ${src}.`);
  });
});

describe("collections: scaffolds / import", () => {
  test("getScaffolds returns auth/base/view templates", async () => {
    const s = await pb.collections.getScaffolds();
    expect(Object.keys(s).sort()).toEqual(["auth", "base", "view"]);
    expect(s.base.type).toBe("base");
    expect(s.base.id).toBe("");
    expect(s.base.name).toBe("");
    expect(s.base.fields.map((f: any) => f.name)).toEqual(["id"]);
    expect(s.auth.type).toBe("auth");
    expect(s.auth.fields.map((f: any) => f.name)).toEqual(["id", "password", "tokenKey", "email", "emailVisibility", "verified"]);
    expect(s.auth.passwordAuth).toEqual({ enabled: true, identityFields: ["email"] });
    expect(s.auth.indexes.length).toBe(2);
    expect(s.auth.mfa).toEqual({ enabled: false, duration: 600, rule: "" });
    expect(s.auth.otp).toMatchObject({ enabled: false, duration: 180, length: 8 });
    expect(s.view.type).toBe("view");
    expect(s.view.viewQuery).toBe("");
  });

  test("import creates + updates collections, deleteMissing removes omitted ones", async () => {
    const keep = track(uniq("imp_keep"));
    const gone = uniq("imp_gone");
    const fresh = track(uniq("imp_fresh"));
    const keepCol = await pb.collections.create({ name: keep, type: "base", fields: [{ name: "a", type: "text" }] });
    await pb.collections.create({ name: gone, type: "base", fields: [] });

    // deleteMissing=false: only creates/updates
    let ok = await pb.collections.import(
      [
        { ...keepCol, fields: [...keepCol.fields, { name: "b", type: "bool" }] },
        { name: fresh, type: "base", fields: [{ name: "x", type: "number" }] },
      ] as never,
      false,
    );
    expect(ok).toBe(true);
    const keepAfter = await pb.collections.getOne(keep);
    expect(keepAfter.fields.map((f: any) => f.name)).toEqual(["id", "a", "b"]);
    expect((await pb.collections.getOne(fresh)).fields.map((f: any) => f.name)).toEqual(["id", "x"]);
    expect((await pb.collections.getOne(gone)).name).toBe(gone);

    // deleteMissing=true: pass everything currently existing except `gone`.
    const all = await pb.collections.getFullList();
    ok = await pb.collections.import(
      all.filter((c) => c.name !== gone),
      true,
    );
    expect(ok).toBe(true);
    const err = await expectError(() => pb.collections.getOne(gone));
    expect(err.status).toBe(404);
    // system collections survive deleteMissing
    expect((await pb.collections.getOne("_superusers")).system).toBe(true);
  });

  test("import with an invalid collection fails atomically", async () => {
    const good = uniq("imp_atomic");
    const err = await expectError(() =>
      pb.collections.import([
        { name: good, type: "base", fields: [] },
        { name: "bad name!", type: "base", fields: [] },
      ] as never),
    );
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to import collections.");
    expect(err.response.data.collections).toBeDefined();
    const notCreated = await expectError(() => pb.collections.getOne(good));
    expect(notCreated.status).toBe(404);
  });
});

describe("collections: authorization", () => {
  test("non-superuser gets 401/403", async () => {
    const { client } = await import("./harness");
    const anon = client();
    const err = await expectError(() => anon.collections.getList());
    expect(err.status).toBe(401);
    expect(err.response.message).toBe("The request requires valid record authorization token.");
    expect(err.response.data).toEqual({});
  });
});
