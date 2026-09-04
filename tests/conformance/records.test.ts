import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import type { RecordModel } from "pocketbase";
import { adminClient, client, dropCollection, expectError, uniq } from "./harness";

/**
 * Records API: /api/collections/{c}/records
 * Fixture: authors <- posts (author relation, tags multi-select, cover file)
 *          posts   <- comments (post relation) enabling `comments_via_post`
 *          members join table for the `@collection.X` rule test
 */
let pb: PocketBase;
const AUTHORS = uniq("authors");
const POSTS = uniq("posts");
const COMMENTS = uniq("comments");
const USERS = uniq("users");
const TEAMS = uniq("teams");
const MEMBERS = uniq("members");
let authorsId = "";
let postsId = "";
let usersId = "";
let alice: RecordModel, bob: RecordModel;
let p1: RecordModel, p2: RecordModel, p3: RecordModel;

const DATE_RE = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}Z$/;

beforeAll(async () => {
  pb = await adminClient();

  const authors = await pb.collections.create({
    name: AUTHORS,
    type: "base",
    listRule: "",
    viewRule: "",
    fields: [
      { name: "name", type: "text", required: true },
      { name: "created", type: "autodate", onCreate: true, onUpdate: false },
    ],
  });
  authorsId = authors.id;

  const posts = await pb.collections.create({
    name: POSTS,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: "",
    updateRule: "",
    deleteRule: "",
    fields: [
      { name: "title", type: "text", required: true, min: 3, max: 20 },
      { name: "content", type: "editor" },
      { name: "views", type: "number", onlyInt: true, min: 0, max: 1000 },
      { name: "score", type: "number" },
      { name: "author", type: "relation", collectionId: authors.id, maxSelect: 1 },
      { name: "tags", type: "select", values: ["go", "rust", "js", "sql"], maxSelect: 2 },
      { name: "category", type: "select", values: ["news", "blog"], maxSelect: 1 },
      { name: "published", type: "bool" },
      { name: "when", type: "date", min: "2020-01-01 00:00:00.000Z", max: "2030-01-01 00:00:00.000Z" },
      { name: "meta", type: "json" },
      { name: "contact", type: "email" },
      { name: "site", type: "url" },
      { name: "slug", type: "text" },
      { name: "cover", type: "file", maxSelect: 3, maxSize: 10000 },
      { name: "created", type: "autodate", onCreate: true, onUpdate: false },
      { name: "updated", type: "autodate", onCreate: true, onUpdate: true },
    ],
    indexes: [`CREATE UNIQUE INDEX \`idx_${POSTS}_slug\` ON \`${POSTS}\` (\`slug\`) WHERE \`slug\` != ''`],
  });
  postsId = posts.id;

  await pb.collections.create({
    name: COMMENTS,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: "",
    fields: [
      { name: "post", type: "relation", collectionId: posts.id, maxSelect: 1, cascadeDelete: true },
      { name: "text", type: "text" },
      { name: "created", type: "autodate", onCreate: true, onUpdate: false },
    ],
  });

  alice = await pb.collection(AUTHORS).create({ name: "Alice" });
  bob = await pb.collection(AUTHORS).create({ name: "Bob" });
  p1 = await pb.collection(POSTS).create({ title: "First post", content: "<p>Hello <b>world</b>, this is a long body text.</p>", views: 10, author: alice.id, tags: ["go", "rust"], category: "news", published: true, when: "2024-01-01 10:00:00.000Z", meta: { a: 1 } });
  await Bun.sleep(5);
  p2 = await pb.collection(POSTS).create({ title: "Second post", views: 20, author: bob.id, tags: ["js"], category: "blog", published: false, when: "2025-06-15 10:00:00.000Z" });
  await Bun.sleep(5);
  p3 = await pb.collection(POSTS).create({ title: "Third post", views: 30, author: alice.id, tags: [], published: true, when: "2021-03-03 10:00:00.000Z" });
  await pb.collection(COMMENTS).create({ post: p1.id, text: "nice" });
  await pb.collection(COMMENTS).create({ post: p1.id, text: "great" });
  await pb.collection(COMMENTS).create({ post: p2.id, text: "meh" });
});

afterAll(async () => {
  for (const n of [MEMBERS, TEAMS, COMMENTS, POSTS, AUTHORS, USERS]) await dropCollection(pb, n);
});

describe("records: CRUD basics", () => {
  test("create returns the full record incl. system fields", async () => {
    const r = await pb.collection(POSTS).create({ title: "Created", views: 1 });
    expect(r.id).toMatch(/^[a-z0-9]{15}$/);
    expect(r.collectionId).toBe(postsId);
    expect(r.collectionName).toBe(POSTS);
    expect(r.title).toBe("Created");
    expect(r.views).toBe(1);
    expect(r.score).toBe(0);
    expect(r.published).toBe(false);
    expect(r.content).toBe("");
    expect(r.author).toBe("");
    expect(r.tags).toEqual([]);
    expect(r.category).toBe("");
    expect(r.when).toBe("");
    expect(r.meta).toBeNull();
    expect(r.cover).toEqual([]);
    expect(r.created).toMatch(DATE_RE);
    expect(r.updated).toMatch(DATE_RE);
    await pb.collection(POSTS).delete(r.id);
  });

  test("client-supplied id is honoured; invalid id rejected", async () => {
    const id = "abcdefghij" + Math.random().toString(36).slice(2, 7);
    const r = await pb.collection(POSTS).create({ id, title: "With id" });
    expect(r.id).toBe(id);
    await pb.collection(POSTS).delete(id);

    const err = await expectError(() => pb.collection(POSTS).create({ id: "TOO-SHORT", title: "Bad id" }));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to create record.");
    // the id is validated like any text field (min=max=15, pattern ^[a-z0-9]+$)
    expect(err.response.data.id.code).toBe("validation_min_text_constraint");
    expect(err.response.data.id.message).toBe("Must be at least 15 character(s).");
    const err2 = await expectError(() => pb.collection(POSTS).create({ id: "abcdefghijklmnO", title: "Bad id" }));
    expect(err2.response.data.id.code).toBe("validation_invalid_format");
    expect(err2.response.data.id.message).toBe("Invalid value format.");
  });

  test("getOne / update / delete / 404", async () => {
    const r = await pb.collection(POSTS).create({ title: "Temp", views: 5 });
    const got = await pb.collection(POSTS).getOne(r.id);
    expect(got.id).toBe(r.id);
    const upd = await pb.collection(POSTS).update(r.id, { views: 6 });
    expect(upd.views).toBe(6);
    expect(upd.title).toBe("Temp");
    expect(upd.updated >= r.updated).toBe(true);
    expect(await pb.collection(POSTS).delete(r.id)).toBe(true);
    const err = await expectError(() => pb.collection(POSTS).getOne(r.id));
    expect(err.status).toBe(404);
    expect(err.response.message).toBe("The requested resource wasn't found.");
    expect(err.response.data).toEqual({});
    const err2 = await expectError(() => pb.collection(POSTS).delete(r.id));
    expect(err2.status).toBe(404);
  });

  test("getOne with empty id -> 404", async () => {
    const err = await expectError(() => pb.collection(POSTS).getOne(""));
    expect(err.status).toBe(404);
  });

  test("update with unknown field is silently ignored", async () => {
    const r = await pb.collection(POSTS).update(p3.id, { doesNotExist: "x" });
    expect(r).not.toHaveProperty("doesNotExist");
  });

  test("number field: onlyInt/min/max; string coercion", async () => {
    const r = await pb.collection(POSTS).create({ title: "Coerce", views: "42" });
    expect(r.views).toBe(42);
    await pb.collection(POSTS).delete(r.id);
  });

  test("delete cascades to comments (cascadeDelete=true)", async () => {
    const post = await pb.collection(POSTS).create({ title: "Cascade" });
    const c = await pb.collection(COMMENTS).create({ post: post.id, text: "x" });
    await pb.collection(POSTS).delete(post.id);
    const err = await expectError(() => pb.collection(COMMENTS).getOne(c.id));
    expect(err.status).toBe(404);
  });

  test("deleting a referenced record without cascade clears the relation", async () => {
    const a = await pb.collection(AUTHORS).create({ name: "Temp author" });
    const post = await pb.collection(POSTS).create({ title: "Ref post", author: a.id });
    await pb.collection(AUTHORS).delete(a.id);
    const again = await pb.collection(POSTS).getOne(post.id);
    expect(again.author).toBe("");
    await pb.collection(POSTS).delete(post.id);
  });
});

describe("records: list / pagination / sort", () => {
  test("pagination envelope", async () => {
    const page = await pb.collection(POSTS).getList(1, 2, { sort: "created" });
    expect(Object.keys(page).sort()).toEqual(["items", "page", "perPage", "totalItems", "totalPages"]);
    expect(page.page).toBe(1);
    expect(page.perPage).toBe(2);
    expect(page.totalItems).toBe(3);
    expect(page.totalPages).toBe(2);
    expect(page.items).toHaveLength(2);
    const page2 = await pb.collection(POSTS).getList(2, 2, { sort: "created" });
    expect(page2.items).toHaveLength(1);
    expect(page2.items[0]!.id).toBe(p3.id);
  });

  test("perPage is capped at 1000 and page < 1 becomes 1", async () => {
    const page = await pb.collection(POSTS).getList(0, 5000);
    expect(page.page).toBe(1);
    expect(page.perPage).toBe(1000);
  });

  test("skipTotal returns -1 for totalItems/totalPages", async () => {
    const page = await pb.collection(POSTS).getList(1, 1, { skipTotal: true });
    expect(page.totalItems).toBe(-1);
    expect(page.totalPages).toBe(-1);
    expect(page.items).toHaveLength(1);
  });

  test("getFullList / getFirstListItem", async () => {
    const all = await pb.collection(POSTS).getFullList({ sort: "title", batch: 2 });
    expect(all.map((r) => r.title)).toEqual(["First post", "Second post", "Third post"]);
    const first = await pb.collection(POSTS).getFirstListItem(`views > 15`, { sort: "views" });
    expect(first.id).toBe(p2.id);
    const err = await expectError(() => pb.collection(POSTS).getFirstListItem(`views > 9999`));
    expect(err.status).toBe(404);
    expect(err.response.message).toBe("The requested resource wasn't found.");
  });

  test("sort: -created, multi-key, relation field, @random", async () => {
    const desc = await pb.collection(POSTS).getFullList({ sort: "-created" });
    expect(desc.map((r) => r.id)).toEqual([p3.id, p2.id, p1.id]);
    const multi = await pb.collection(POSTS).getFullList({ sort: "-published,views" });
    expect(multi.map((r) => r.id)).toEqual([p1.id, p3.id, p2.id]);
    const byAuthor = await pb.collection(POSTS).getFullList({ sort: "author.name,-views" });
    expect(byAuthor.map((r) => r.id)).toEqual([p3.id, p1.id, p2.id]);
    const rnd = await pb.collection(POSTS).getFullList({ sort: "@random" });
    expect(rnd).toHaveLength(3);
    const byId = await pb.collection(POSTS).getFullList({ sort: "+id" });
    expect(byId.map((r) => r.id)).toEqual([...byId.map((r) => r.id)].sort());
  });

  test("sort by unknown field -> 400", async () => {
    const err = await expectError(() => pb.collection(POSTS).getList(1, 1, { sort: "nope" }));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Something went wrong while processing your request.");
    expect(err.response.data).toEqual({});
  });
});

describe("records: filter", () => {
  const list = (filter: string, extra: Record<string, unknown> = {}) =>
    pb.collection(POSTS).getFullList({ filter, sort: "created", ...extra }).then((r) => r.map((x) => x.id));

  test("comparison operators", async () => {
    expect(await list(`views = 20`)).toEqual([p2.id]);
    expect(await list(`views != 20`)).toEqual([p1.id, p3.id]);
    expect(await list(`views > 10`)).toEqual([p2.id, p3.id]);
    expect(await list(`views >= 10 && views < 30`)).toEqual([p1.id, p2.id]);
    expect(await list(`views <= 10 || views = 30`)).toEqual([p1.id, p3.id]);
    expect(await list(`published = true`)).toEqual([p1.id, p3.id]);
    expect(await list(`published = false`)).toEqual([p2.id]);
    expect(await list(`(views = 10 || views = 20) && published = true`)).toEqual([p1.id]);
  });

  test("= is case-sensitive; ~ (LIKE) is case-insensitive; :lower normalises", async () => {
    expect(await list(`title = "first post"`)).toEqual([]);
    expect(await list(`title = "First post"`)).toEqual([p1.id]);
    expect(await list(`title ~ "first"`)).toEqual([p1.id]);
    expect(await list(`title:lower = "first post"`)).toEqual([p1.id]);
  });

  test("~ with and without explicit %", async () => {
    expect(await list(`title ~ "post"`)).toEqual([p1.id, p2.id, p3.id]);
    expect(await list(`title ~ "Sec"`)).toEqual([p2.id]);
    expect(await list(`title ~ "Sec%"`)).toEqual([p2.id]);
    expect(await list(`title ~ "%ond%"`)).toEqual([p2.id]);
    expect(await list(`title ~ "ond%"`)).toEqual([]);
    expect(await list(`title !~ "First"`)).toEqual([p2.id, p3.id]);
  });

  test("multi-select column: plain operators see the raw JSON text, :each unpacks it", async () => {
    // NOTE (PocketBase quirk): the `?` "any" prefix only changes semantics for
    // joined identifiers (relations, back-relations, @collection). On a plain
    // multi-select column `tags ?= "rust"` is compared against the JSON text
    // '["go","rust"]' and therefore never matches. Use `tags:each ?= "rust"`.
    expect(await list(`tags ?= "rust"`)).toEqual([]);
    expect(await list(`tags = "rust"`)).toEqual([]);
    expect(await list(`tags ?!= "go"`)).toEqual([p1.id, p2.id, p3.id]);
    expect(await list(`tags ~ "rust"`)).toEqual([p1.id]);
    expect(await list(`tags ?~ "s"`)).toEqual([p1.id, p2.id]);
    // :each + ?= is the "at least one element equals" form; :each + = means "every element equals"
    expect(await list(`tags:each ?= "rust"`)).toEqual([p1.id]);
    expect(await list(`tags:each ?= "js"`)).toEqual([p2.id]);
    expect(await list(`tags:each = "rust"`)).toEqual([]);
    expect(await list(`tags:each = "js"`)).toEqual([p2.id]);
  });

  test("empty / null checks", async () => {
    expect(await list(`author = ""`)).toEqual([]);
    expect(await list(`category = ""`)).toEqual([p3.id]);
    expect(await list(`category != ""`)).toEqual([p1.id, p2.id]);
    expect(await list(`meta = null`)).toEqual([p2.id, p3.id]);
    expect(await list(`tags:length = 0`)).toEqual([p3.id]);
  });

  test(":length / :each / :lower modifiers", async () => {
    expect(await list(`tags:length = 2`)).toEqual([p1.id]);
    expect(await list(`tags:length > 0`)).toEqual([p1.id, p2.id]);
    expect(await list(`tags:each != "go"`)).toEqual([p2.id, p3.id]);
    expect(await list(`tags:each ~ "s"`)).toEqual([p2.id]);
    expect(await list(`title:lower = "second post"`)).toEqual([p2.id]);
    expect(await list(`title:lower ~ "second"`)).toEqual([p2.id]);
  });

  test("relation field filtering (dot notation) and back-relation", async () => {
    expect(await list(`author.name = "Alice"`)).toEqual([p1.id, p3.id]);
    expect(await list(`author.name ~ "bo"`)).toEqual([p2.id]);
    expect(await list(`${COMMENTS}_via_post.text ?= "nice"`)).toEqual([p1.id]);
    expect(await list(`${COMMENTS}_via_post.text ?~ "e"`)).toEqual([p1.id, p2.id]);
    // a back-relation with no rows is NULL, so compare with "" rather than :length = 0
    expect(await list(`${COMMENTS}_via_post.id:length = 0`)).toEqual([]);
    expect(await list(`${COMMENTS}_via_post.id:length > 0`)).toEqual([p1.id, p2.id]);
    expect(await list(`${COMMENTS}_via_post.id = ""`)).toEqual([p3.id]);
    // without `?` every joined row must satisfy the condition (p1 has "nice" AND "great")
    expect(await list(`${COMMENTS}_via_post.text = "nice"`)).toEqual([]);
    expect(await list(`${COMMENTS}_via_post.text = "meh"`)).toEqual([p2.id]);
    expect(await list(`${COMMENTS}_via_post.text ?!= "nice"`)).toEqual([p1.id, p2.id, p3.id]);
    expect(await list(`${COMMENTS}_via_post.text != "nice"`)).toEqual([p2.id, p3.id]);
  });

  test("json field filtering", async () => {
    expect(await list(`meta.a = 1`)).toEqual([p1.id]);
    expect(await list(`meta.a != 1`)).toEqual([p2.id, p3.id]);
  });

  test("date macros @now / @todayStart / @todayEnd / @yearStart", async () => {
    expect(await list(`created < @now`)).toEqual([p1.id, p2.id, p3.id]);
    expect(await list(`created >= @todayStart && created <= @todayEnd`)).toEqual([p1.id, p2.id, p3.id]);
    expect(await list(`when < @yearStart`)).toEqual([p1.id, p2.id, p3.id]);
    expect(await list(`when > @yearStart`)).toEqual([]);
    expect(await list(`when > @now`)).toEqual([]);
    expect(await list(`when >= "2024-01-01 00:00:00.000Z"`)).toEqual([p1.id, p2.id]);
    expect(await list(`when >= "2024-01-01"`)).toEqual([p1.id, p2.id]);
  });

  test("invalid filter -> 400 generic message", async () => {
    const err = await expectError(() => pb.collection(POSTS).getList(1, 1, { filter: "title = " }));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Something went wrong while processing your request.");
    const err2 = await expectError(() => pb.collection(POSTS).getList(1, 1, { filter: "unknownField = 1" }));
    expect(err2.status).toBe(400);
  });

  test("pb.filter() parameter binding", async () => {
    const f = pb.filter("title = {:t} && views >= {:v}", { t: "Second post", v: 20 });
    expect(await list(f)).toEqual([p2.id]);
  });
});

describe("records: expand", () => {
  test("single relation expand", async () => {
    const r = await pb.collection(POSTS).getOne(p1.id, { expand: "author" });
    expect(r.author).toBe(alice.id);
    expect(r.expand?.author?.id).toBe(alice.id);
    expect(r.expand?.author?.name).toBe("Alice");
    expect(r.expand?.author?.collectionName).toBe(AUTHORS);
  });

  test("no expand -> no `expand` key; empty relation -> no entry", async () => {
    const r = await pb.collection(POSTS).getOne(p1.id);
    expect(r).not.toHaveProperty("expand");
    const noAuthor = await pb.collection(POSTS).create({ title: "No author" });
    const r2 = await pb.collection(POSTS).getOne(noAuthor.id, { expand: "author" });
    expect(r2.expand ?? {}).toEqual({});
    await pb.collection(POSTS).delete(noAuthor.id);
  });

  test("multi expand, nested expand, back-relation expand", async () => {
    const c = await pb.collection(COMMENTS).getFirstListItem(`text = "nice"`, { expand: "post,post.author" });
    expect(c.expand?.post?.id).toBe(p1.id);
    expect(c.expand?.post?.expand?.author?.name).toBe("Alice");

    const back = await pb.collection(POSTS).getOne(p1.id, { expand: `${COMMENTS}_via_post` });
    const comments = back.expand?.[`${COMMENTS}_via_post`];
    expect(Array.isArray(comments)).toBe(true);
    expect(comments.map((x: RecordModel) => x.text).sort()).toEqual(["great", "nice"]);

    const list = await pb.collection(POSTS).getFullList({ expand: "author", sort: "created" });
    expect(list.map((r) => r.expand?.author?.name)).toEqual(["Alice", "Bob", "Alice"]);
  });

  test("expand of unknown relation is ignored", async () => {
    const r = await pb.collection(POSTS).getOne(p1.id, { expand: "nope" });
    expect(r.expand ?? {}).toEqual({});
  });
});

describe("records: fields", () => {
  test("fields=id,title", async () => {
    const r = await pb.collection(POSTS).getOne(p1.id, { fields: "id,title" });
    expect(Object.keys(r).sort()).toEqual(["id", "title"]);
  });

  test("fields with wildcard and expand path", async () => {
    const r = await pb.collection(POSTS).getOne(p1.id, { expand: "author", fields: "*,expand.author.name" });
    expect(r.title).toBe("First post");
    expect(r.expand?.author).toEqual({ name: "Alice" });
    const r2 = await pb.collection(POSTS).getOne(p1.id, { expand: "author", fields: "id,expand.author.*" });
    expect(Object.keys(r2).sort()).toEqual(["expand", "id"]);
    expect(r2.expand?.author?.name).toBe("Alice");
    expect(r2.expand?.author?.id).toBe(alice.id);
  });

  test(":excerpt(max,withEllipsis) modifier strips html", async () => {
    const r = await pb.collection(POSTS).getOne(p1.id, { fields: "content:excerpt(10,true)" });
    expect(r.content).toBe("Hello worl...");
    const r2 = await pb.collection(POSTS).getOne(p1.id, { fields: "content:excerpt(10)" });
    expect(r2.content).toBe("Hello worl");
    const r3 = await pb.collection(POSTS).getOne(p1.id, { fields: "content:excerpt(200,true)" });
    expect(r3.content).toBe("Hello world, this is a long body text.");
  });

  test("fields in list responses", async () => {
    const page = await pb.collection(POSTS).getList(1, 5, { fields: "id" });
    for (const it of page.items) expect(Object.keys(it)).toEqual(["id"]);
  });
});

describe("records: modifiers", () => {
  test("+/- on multi select (append, prepend, remove)", async () => {
    const r = await pb.collection(POSTS).create({ title: "Mods", tags: ["go"] });
    let u = await pb.collection(POSTS).update(r.id, { "tags+": "rust" });
    expect(u.tags).toEqual(["go", "rust"]);
    u = await pb.collection(POSTS).update(r.id, { "tags-": "go" });
    expect(u.tags).toEqual(["rust"]);
    u = await pb.collection(POSTS).update(r.id, { "+tags": "js" });
    expect(u.tags).toEqual(["js", "rust"]);
    // exceeding maxSelect via + fails
    const err = await expectError(() => pb.collection(POSTS).update(r.id, { "tags+": "sql" }));
    expect(err.status).toBe(400);
    expect(err.response.data.tags.code).toBe("validation_too_many_values");
    await pb.collection(POSTS).delete(r.id);
  });

  test("+/- on number fields", async () => {
    const r = await pb.collection(POSTS).create({ title: "Counter", views: 10 });
    let u = await pb.collection(POSTS).update(r.id, { "views+": 5 });
    expect(u.views).toBe(15);
    u = await pb.collection(POSTS).update(r.id, { "views-": 3 });
    expect(u.views).toBe(12);
    await pb.collection(POSTS).delete(r.id);
  });

  test("+/- on multi relation", async () => {
    const name = uniq("multirel");
    const col = await pb.collections.create({
      name,
      type: "base",
      fields: [{ name: "authors", type: "relation", collectionId: authorsId, maxSelect: 5 }],
    });
    try {
      const r = await pb.collection(name).create({ authors: [alice.id] });
      let u = await pb.collection(name).update(r.id, { "authors+": bob.id });
      expect(u.authors).toEqual([alice.id, bob.id]);
      u = await pb.collection(name).update(r.id, { "authors-": alice.id });
      expect(u.authors).toEqual([bob.id]);
    } finally {
      await pb.collections.delete(col.id);
    }
  });
});

describe("records: multipart / @jsonPayload", () => {
  test("@jsonPayload merges JSON body with multipart file", async () => {
    const fd = new FormData();
    fd.append("@jsonPayload", JSON.stringify({ title: "Multipart", tags: ["go", "js"], meta: { k: "v" }, views: 3 }));
    fd.append("cover", new File(["hello"], "hello.txt", { type: "text/plain" }));
    const r = await pb.collection(POSTS).create(fd);
    expect(r.title).toBe("Multipart");
    expect(r.tags).toEqual(["go", "js"]);
    expect(r.meta).toEqual({ k: "v" });
    expect(r.views).toBe(3);
    expect(r.cover).toHaveLength(1);
    expect(r.cover[0]).toMatch(/^hello_[a-z0-9]{10}\.txt$/);
    await pb.collection(POSTS).delete(r.id);
  });

  test("plain multipart fields: repeated keys become arrays, json field parsed", async () => {
    const fd = new FormData();
    fd.append("title", "Plain mp");
    fd.append("tags", "go");
    fd.append("tags", "rust");
    fd.append("meta", '{"x":1}');
    fd.append("published", "true");
    const r = await pb.collection(POSTS).create(fd);
    expect(r.tags).toEqual(["go", "rust"]);
    expect(r.meta).toEqual({ x: 1 });
    expect(r.published).toBe(true);
    await pb.collection(POSTS).delete(r.id);
  });
});

describe("records: validation errors", () => {
  test("required", async () => {
    const err = await expectError(() => pb.collection(POSTS).create({}));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to create record.");
    expect(err.response.data.title).toEqual({ code: "validation_required", message: "Cannot be blank." });
  });

  test("text min / max", async () => {
    let err = await expectError(() => pb.collection(POSTS).create({ title: "ab" }));
    expect(err.response.data.title.code).toBe("validation_min_text_constraint");
    expect(err.response.data.title.message).toBe("Must be at least 3 character(s).");
    err = await expectError(() => pb.collection(POSTS).create({ title: "x".repeat(21) }));
    expect(err.response.data.title.code).toBe("validation_max_text_constraint");
    expect(err.response.data.title.message).toBe("Must be no more than 20 character(s).");
  });

  test("number onlyInt / min / max / NaN", async () => {
    let err = await expectError(() => pb.collection(POSTS).create({ title: "Num", views: 1.5 }));
    expect(err.response.data.views.code).toBe("validation_only_int_constraint");
    expect(err.response.data.views.message).toBe("Decimal numbers are not allowed.");
    err = await expectError(() => pb.collection(POSTS).create({ title: "Num", views: -1 }));
    expect(err.response.data.views.code).toBe("validation_min_number_constraint");
    err = await expectError(() => pb.collection(POSTS).create({ title: "Num", views: 1001 }));
    expect(err.response.data.views.code).toBe("validation_max_number_constraint");
    // non-numeric strings are silently coerced to 0
    const r = await pb.collection(POSTS).create({ title: "Num", views: "abc" });
    expect(r.views).toBe(0);
    await pb.collection(POSTS).delete(r.id);
  });

  test("select: value not in list / too many values", async () => {
    let err = await expectError(() => pb.collection(POSTS).create({ title: "Sel", category: "nope" }));
    expect(err.response.data.category.code).toBe("validation_invalid_value");
    err = await expectError(() => pb.collection(POSTS).create({ title: "Sel", tags: ["go", "rust", "js"] }));
    expect(err.response.data.tags.code).toBe("validation_too_many_values");
    expect(err.response.data.tags.message).toBe("Select no more than 2.");
    // a single select given an array keeps the LAST element instead of failing
    const r = await pb.collection(POSTS).create({ title: "Sel", category: ["news", "blog"] });
    expect(r.category).toBe("blog");
    await pb.collection(POSTS).delete(r.id);
  });

  test("relation: missing record / wrong collection", async () => {
    let err = await expectError(() => pb.collection(POSTS).create({ title: "Rel", author: "doesnotexist000" }));
    expect(err.response.data.author.code).toBe("validation_missing_rel_records");
    expect(err.response.data.author.message).toBe("Failed to find all relation records with the provided ids.");
    err = await expectError(() => pb.collection(POSTS).create({ title: "Rel", author: p1.id }));
    expect(err.response.data.author.code).toBe("validation_missing_rel_records");
  });

  test("email / url format", async () => {
    let err = await expectError(() => pb.collection(POSTS).create({ title: "Fmt", contact: "not-an-email" }));
    expect(err.response.data.contact.code).toBe("validation_is_email");
    expect(err.response.data.contact.message).toBe("Must be a valid email address.");
    err = await expectError(() => pb.collection(POSTS).create({ title: "Fmt", site: "not a url" }));
    expect(err.response.data.site.code).toBe("validation_invalid_url");
    expect(err.response.data.site.message).toBe("Must be a valid url.");
  });

  test("date min / max; unparsable dates are silently stored as empty", async () => {
    let err = await expectError(() => pb.collection(POSTS).create({ title: "Date test", when: "2010-01-01 00:00:00.000Z" }));
    expect(err.response.data.when.code).toBe("validation_min_greater_equal_than_required");
    expect(err.response.data.when.message).toBe("Must be no less than 2020-01-01 00:00:00 +0000 UTC.");
    expect(err.response.data.when.params).toEqual({ threshold: "2020-01-01T00:00:00Z" });
    err = await expectError(() => pb.collection(POSTS).create({ title: "Date test", when: "2040-01-01 00:00:00.000Z" }));
    expect(err.response.data.when.code).toBe("validation_max_less_equal_than_required");
    expect(err.response.data.when.message).toBe("Must be no greater than 2030-01-01 00:00:00 +0000 UTC.");
    // no validation error for garbage: it is coerced to the zero date ("")
    const r = await pb.collection(POSTS).create({ title: "Date test", when: "not a date" });
    expect(r.when).toBe("");
    const r2 = await pb.collection(POSTS).update(r.id, { when: "2024-05-05" });
    expect(r2.when).toBe("2024-05-05 00:00:00.000Z");
    await pb.collection(POSTS).delete(r.id);
  });

  test("date input accepts RFC3339 and normalises output", async () => {
    const r = await pb.collection(POSTS).create({ title: "Dtfmt", when: "2024-05-05T12:30:00Z" });
    expect(r.when).toBe("2024-05-05 12:30:00.000Z");
    const r2 = await pb.collection(POSTS).update(r.id, { when: "2024-05-05T12:30:00+02:00" });
    expect(r2.when).toBe("2024-05-05 10:30:00.000Z");
    await pb.collection(POSTS).delete(r.id);
  });

  test("unique index -> validation_not_unique", async () => {
    const a = await pb.collection(POSTS).create({ title: "Uniq", slug: "same-slug" });
    const err = await expectError(() => pb.collection(POSTS).create({ title: "Uniq", slug: "same-slug" }));
    expect(err.status).toBe(400);
    expect(err.response.data.slug).toEqual({ code: "validation_not_unique", message: "Value must be unique." });
    await pb.collection(POSTS).delete(a.id);
  });

  test("multiple errors are reported together", async () => {
    const err = await expectError(() => pb.collection(POSTS).create({ title: "x", views: 5000, contact: "bad" }));
    expect(Object.keys(err.response.data).sort()).toEqual(["contact", "title", "views"]);
  });

  test("malformed JSON body", async () => {
    const res = await fetch(`${pb.baseURL}/api/collections/${POSTS}/records`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: pb.authStore.token },
      body: "{not json",
    });
    expect(res.status).toBe(400);
    const body = await res.json();
    // records endpoints use the generic 400 message for unparsable bodies
    expect(body.message).toBe("Something went wrong while processing your request.");
    expect(body.data).toEqual({});
  });
});

describe("records: API rules", () => {
  let user1: PocketBase, user2: PocketBase;
  let u1: RecordModel, u2: RecordModel;
  let team1: RecordModel, team2: RecordModel;

  beforeAll(async () => {
    const users = await pb.collections.create({ name: USERS, type: "auth", fields: [] });
    usersId = users.id;
    u1 = await pb.collection(USERS).create({ email: `u1_${USERS}@test.local`, password: "password123", passwordConfirm: "password123" });
    u2 = await pb.collection(USERS).create({ email: `u2_${USERS}@test.local`, password: "password123", passwordConfirm: "password123" });
    user1 = client();
    await user1.collection(USERS).authWithPassword(u1.email, "password123");
    user2 = client();
    await user2.collection(USERS).authWithPassword(u2.email, "password123");

    const teams = await pb.collections.create({
      name: TEAMS,
      type: "base",
      fields: [{ name: "name", type: "text" }],
    });

    await pb.collections.create({
      name: MEMBERS,
      type: "base",
      fields: [
        { name: "team", type: "relation", collectionId: teams.id, maxSelect: 1 },
        { name: "user", type: "relation", collectionId: users.id, maxSelect: 1 },
      ],
    });
    // A user can list/view teams they are a member of (join through another collection)
    await pb.collections.update(teams.id, {
      listRule: `@collection.${MEMBERS}.user ?= @request.auth.id && @collection.${MEMBERS}.team ?= id`,
      viewRule: `@collection.${MEMBERS}.user ?= @request.auth.id && @collection.${MEMBERS}.team ?= id`,
    });
    team1 = await pb.collection(TEAMS).create({ name: "T1" });
    team2 = await pb.collection(TEAMS).create({ name: "T2" });
    await pb.collection(MEMBERS).create({ team: team1.id, user: u1.id });
    await pb.collection(MEMBERS).create({ team: team2.id, user: u2.id });
  });

  test("null rule -> superuser only (403 for guests and users)", async () => {
    const anon = client();
    let err = await expectError(() => anon.collection(MEMBERS).getList());
    expect(err.status).toBe(403);
    expect(err.response.message).toBe("Only superusers can perform this action.");
    expect(err.response.data).toEqual({});
    err = await expectError(() => user1.collection(MEMBERS).getList());
    expect(err.status).toBe(403);
    // superuser works
    expect((await pb.collection(MEMBERS).getList()).totalItems).toBe(2);
  });

  test("@collection.X join rule", async () => {
    const t1 = await user1.collection(TEAMS).getFullList();
    expect(t1.map((t) => t.id)).toEqual([team1.id]);
    const t2 = await user2.collection(TEAMS).getFullList();
    expect(t2.map((t) => t.id)).toEqual([team2.id]);
    expect((await client().collection(TEAMS).getFullList()).length).toBe(0);
    // viewRule failing -> 404 (not 403)
    const err = await expectError(() => user1.collection(TEAMS).getOne(team2.id));
    expect(err.status).toBe(404);
  });

  test("@request.auth.id ownership rule + @request.body in createRule", async () => {
    const name = uniq("owned");
    await pb.collections.create({
      name,
      type: "base",
      fields: [
        { name: "owner", type: "relation", collectionId: usersId, maxSelect: 1 },
        { name: "title", type: "text" },
        { name: "secret", type: "text" },
      ],
      listRule: "owner = @request.auth.id",
      viewRule: "owner = @request.auth.id",
      createRule: `@request.auth.id != "" && @request.body.owner = @request.auth.id && @request.body.secret:isset = false`,
      updateRule: "owner = @request.auth.id",
      deleteRule: "owner = @request.auth.id",
    });
    try {
      // guest cannot create (rule fails -> 400 on create)
      let err = await expectError(() => client().collection(name).create({ title: "x" }));
      expect(err.status).toBe(400);
      expect(err.response.message).toBe("Failed to create record.");
      expect(err.response.data).toEqual({});
      // wrong owner
      err = await expectError(() => user1.collection(name).create({ title: "x", owner: u2.id }));
      expect(err.status).toBe(400);
      // :isset blocks body keys
      err = await expectError(() => user1.collection(name).create({ title: "x", owner: u1.id, secret: "s" }));
      expect(err.status).toBe(400);

      const mine = await user1.collection(name).create({ title: "mine", owner: u1.id });
      expect(mine.owner).toBe(u1.id);
      expect((await user1.collection(name).getFullList()).length).toBe(1);
      expect((await user2.collection(name).getFullList()).length).toBe(0);
      err = await expectError(() => user2.collection(name).getOne(mine.id));
      expect(err.status).toBe(404);
      err = await expectError(() => user2.collection(name).update(mine.id, { title: "hacked" }));
      expect(err.status).toBe(404);
      err = await expectError(() => user2.collection(name).delete(mine.id));
      expect(err.status).toBe(404);
      const upd = await user1.collection(name).update(mine.id, { title: "still mine" });
      expect(upd.title).toBe("still mine");
      expect(await user1.collection(name).delete(mine.id)).toBe(true);
    } finally {
      await dropCollection(pb, name);
    }
  });

  test("@request.auth relation dot-notation and @request.query in rules", async () => {
    const name = uniq("ruleq");
    await pb.collections.create({
      name,
      type: "base",
      fields: [{ name: "title", type: "text" }],
      listRule: `@request.auth.collectionName = "${USERS}" && @request.query.magic = "42"`,
      viewRule: `@request.auth.email ~ "@test.local"`,
    });
    try {
      const r = await pb.collection(name).create({ title: "q" });
      // a failing listRule is not an error: it simply yields an empty list
      expect((await user1.collection(name).getList()).totalItems).toBe(0);
      expect((await client().collection(name).getList()).totalItems).toBe(0);
      expect((await user1.collection(name).getList(1, 10, { query: { magic: "42" } })).totalItems).toBe(1);
      expect((await user1.collection(name).getOne(r.id)).title).toBe("q");
      const err = await expectError(() => client().collection(name).getOne(r.id));
      expect(err.status).toBe(404);
    } finally {
      await dropCollection(pb, name);
    }
  });

  test("empty-string rule is public; expand respects target viewRule", async () => {
    // MEMBERS is superuser-only; expanding it from TEAMS (public-ish) as a user yields no expand
    const t = await user1.collection(TEAMS).getOne(team1.id, { expand: `${MEMBERS}_via_team` });
    expect(t.expand ?? {}).toEqual({});
    const asAdmin = await pb.collection(TEAMS).getOne(team1.id, { expand: `${MEMBERS}_via_team` });
    expect(asAdmin.expand?.[`${MEMBERS}_via_team`]).toHaveLength(1);
  });
});
