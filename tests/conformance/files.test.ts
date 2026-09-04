import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { adminClient, client, dropCollection, expectError, uniq } from "./harness";

/** Files: upload, getURL, thumbs, protected files + file tokens, download flag. */
let pb: PocketBase;
const DOCS = uniq("docs");
let docsId = "";
const PNG = Uint8Array.from(
  atob(
    // 1x1 transparent png
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  ),
  (c) => c.charCodeAt(0),
);

const txt = (name: string, body = "hello world") => new File([body], name, { type: "text/plain" });
const png = (name = "pixel.png") => new File([PNG], name, { type: "image/png" });

beforeAll(async () => {
  pb = await adminClient();
  const col = await pb.collections.create({
    name: DOCS,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: "",
    updateRule: "",
    deleteRule: "",
    fields: [
      { name: "title", type: "text" },
      { name: "doc", type: "file", maxSelect: 1, maxSize: 5242880, mimeTypes: ["text/plain", "image/png"], thumbs: ["100x100", "50x50t"] },
      { name: "attachments", type: "file", maxSelect: 3, maxSize: 5242880 },
      { name: "secret", type: "file", maxSelect: 1, protected: true },
      { name: "tiny", type: "file", maxSelect: 1, maxSize: 5 },
      { name: "imgOnly", type: "file", maxSelect: 1, mimeTypes: ["image/png"] },
    ],
  });
  docsId = col.id;
});

afterAll(async () => {
  await dropCollection(pb, DOCS);
});

describe("files: upload", () => {
  test("single file via FormData; stored name is <base>_<10 rand>.<ext>", async () => {
    const fd = new FormData();
    fd.append("title", "Single");
    fd.append("doc", txt("notes.txt"));
    const r = await pb.collection(DOCS).create(fd);
    expect(typeof r.doc).toBe("string");
    expect(r.doc).toMatch(/^notes_[a-z0-9]{10}\.txt$/);
    expect(r.attachments).toEqual([]);

    const url = pb.files.getURL(r, r.doc);
    expect(url).toBe(`${pb.baseURL}/api/files/${docsId}/${r.id}/${r.doc}`);
    const res = await fetch(url);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("text/plain");
    expect(await res.text()).toBe("hello world");
    await pb.collection(DOCS).delete(r.id);
  });

  test("multiple files on one field; append with + and remove by name", async () => {
    const fd = new FormData();
    fd.append("title", "Multi");
    fd.append("attachments", txt("a.txt", "A"));
    fd.append("attachments", txt("b.txt", "B"));
    const r = await pb.collection(DOCS).create(fd);
    expect(r.attachments).toHaveLength(2);
    // short basenames get padded with random chars before the `_<10 rand>` suffix
    expect(r.attachments[0]).toMatch(/^a[a-z0-9]*_[a-z0-9]{10}\.txt$/);

    const fd2 = new FormData();
    fd2.append("attachments+", txt("c.txt", "C"));
    const r2 = await pb.collection(DOCS).update(r.id, fd2);
    expect(r2.attachments).toHaveLength(3);

    // remove one by filename
    const r3 = await pb.collection(DOCS).update(r.id, { "attachments-": r2.attachments[0] });
    expect(r3.attachments).toHaveLength(2);
    expect(r3.attachments).not.toContain(r2.attachments[0]);
    // the deleted file is gone from storage
    const gone = await fetch(pb.files.getURL(r, r2.attachments[0]));
    expect(gone.status).toBe(404);

    // clearing the whole field
    const r4 = await pb.collection(DOCS).update(r.id, { attachments: null });
    expect(r4.attachments).toEqual([]);
    await pb.collection(DOCS).delete(r.id);
  });

  test("uploading more than maxSelect fails", async () => {
    const fd = new FormData();
    for (const n of ["a", "b", "c", "d"]) fd.append("attachments", txt(`${n}.txt`));
    const err = await expectError(() => pb.collection(DOCS).create(fd));
    expect(err.status).toBe(400);
    expect(err.response.data.attachments.code).toBe("validation_too_many_files");
  });

  test("maxSize violation -> validation_file_size_limit", async () => {
    const fd = new FormData();
    fd.append("tiny", txt("big.txt", "way too much content"));
    const err = await expectError(() => pb.collection(DOCS).create(fd));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to create record.");
    expect(err.response.data.tiny.code).toBe("validation_file_size_limit");
    expect(err.response.data.tiny.message).toBe("Failed to upload big.txt - the maximum allowed file size is 5 bytes.");
  });

  test("mimeTypes violation -> validation_invalid_mime_type", async () => {
    const fd = new FormData();
    fd.append("imgOnly", txt("nope.txt"));
    const err = await expectError(() => pb.collection(DOCS).create(fd));
    expect(err.status).toBe(400);
    expect(err.response.data.imgOnly.code).toBe("validation_invalid_mime_type");
    expect(err.response.data.imgOnly.message).toContain("image/png");
  });

  test("deleting the record removes its files", async () => {
    const fd = new FormData();
    fd.append("doc", txt("bye.txt"));
    const r = await pb.collection(DOCS).create(fd);
    const url = pb.files.getURL(r, r.doc);
    expect((await fetch(url)).status).toBe(200);
    await pb.collection(DOCS).delete(r.id);
    expect((await fetch(url)).status).toBe(404);
  });
});

describe("files: serving", () => {
  test("thumbs, ?download=1 and unknown file 404", async () => {
    const fd = new FormData();
    fd.append("doc", png());
    const r = await pb.collection(DOCS).create(fd);
    const base = pb.files.getURL(r, r.doc);

    const thumbUrl = pb.files.getURL(r, r.doc, { thumb: "100x100" });
    expect(thumbUrl).toBe(`${base}?thumb=100x100`);
    const thumb = await fetch(thumbUrl);
    expect(thumb.status).toBe(200);
    expect(thumb.headers.get("content-type")).toContain("image/png");
    // a thumb size that is not in the field's `thumbs` list is still generated on demand
    const adhoc = await fetch(pb.files.getURL(r, r.doc, { thumb: "20x20" }));
    expect(adhoc.status).toBe(200);

    const dl = await fetch(`${base}?download=1`);
    expect(dl.status).toBe(200);
    expect(dl.headers.get("content-disposition")).toContain("attachment;");
    expect(dl.headers.get("content-disposition")).toContain(r.doc);
    const inline = await fetch(base);
    expect(inline.headers.get("content-disposition")).toBe(`inline; filename="${r.doc}"`);

    expect((await fetch(`${pb.baseURL}/api/files/${docsId}/${r.id}/missing.txt`)).status).toBe(404);
    expect((await fetch(`${pb.baseURL}/api/files/${docsId}/nonexistent0001/x.txt`)).status).toBe(404);
    await pb.collection(DOCS).delete(r.id);
  });

  test("thumbs are not generated for non-image files", async () => {
    const fd = new FormData();
    fd.append("doc", txt("plain.txt"));
    const r = await pb.collection(DOCS).create(fd);
    const res = await fetch(pb.files.getURL(r, r.doc, { thumb: "100x100" }));
    // PocketBase falls back to the original file
    expect(res.status).toBe(200);
    expect(await res.text()).toBe("hello world");
    await pb.collection(DOCS).delete(r.id);
  });

  test("getURL helpers", async () => {
    expect(pb.files.getURL({ id: "", collectionId: "x" } as never, "f.txt")).toBe("");
    expect(pb.files.getURL({ id: "r1", collectionId: "c1" } as never, "")).toBe("");
    const withToken = pb.files.getURL({ id: "r1", collectionId: "c1" } as never, "f.txt", { token: "tok" });
    expect(withToken).toBe(`${pb.baseURL}/api/files/c1/r1/f.txt?token=tok`);
  });
});

describe("files: protected files and tokens", () => {
  test("protected file on a publicly viewable record is still served without a token", async () => {
    // NOTE: `protected` does not mean "a token is always required" - PocketBase
    // evaluates the collection's viewRule for the token owner, and an empty
    // rule is public, so a protected file in a public collection is readable
    // by anyone. Only a restricted viewRule actually gates the file.
    const fd = new FormData();
    fd.append("secret", txt("classified.txt", "top secret"));
    const r = await pb.collection(DOCS).create(fd);
    const anon = await fetch(pb.files.getURL(r, r.secret));
    expect(anon.status).toBe(200);
    expect(await anon.text()).toBe("top secret");

    const token = await pb.files.getToken();
    expect(typeof token).toBe("string");
    const claims = JSON.parse(Buffer.from(token.split(".")[1]!, "base64url").toString("utf8"));
    expect(claims.type).toBe("file");
    expect(claims.collectionId).toBe("pbc_3142635823"); // the superuser's collection
    expect(claims.id).toBe(pb.authStore.record!.id);
    const ok = await fetch(pb.files.getURL(r, r.secret, { token }));
    expect(ok.status).toBe(200);
    await pb.collection(DOCS).delete(r.id);
  });

  test("getToken requires auth", async () => {
    const err = await expectError(() => client().files.getToken());
    expect(err.status).toBe(401);
    expect(err.response.message).toBe("The request requires valid record authorization token.");
  });

  test("a protected file in a restricted collection needs a token of an authorized viewer", async () => {
    // a private collection: only superusers can view
    const name = uniq("private");
    await pb.collections.create({
      name,
      type: "base",
      fields: [{ name: "secret", type: "file", maxSelect: 1, protected: true }],
    });
    const users = uniq("fileusers");
    await pb.collections.create({ name: users, type: "auth", fields: [] });
    try {
      const fd = new FormData();
      fd.append("secret", txt("s.txt", "hidden"));
      const r = await pb.collection(name).create(fd);
      const u = await pb.collection(users).create({ email: `f@${users}.test`, password: "Password123!", passwordConfirm: "Password123!" });
      const c = client();
      await c.collection(users).authWithPassword(u.email, "Password123!");
      const userToken = await c.files.getToken();
      // an unauthorized viewer (and an anonymous request) get 404, not 403
      expect((await fetch(pb.files.getURL(r, r.secret, { token: userToken }))).status).toBe(404);
      expect((await fetch(pb.files.getURL(r, r.secret))).status).toBe(404);
      expect((await fetch(pb.files.getURL(r, r.secret, { token: "not.a.token" }))).status).toBe(404);
      // the superuser's file token works
      const su = await pb.files.getToken();
      const ok = await fetch(pb.files.getURL(r, r.secret, { token: su }));
      expect(ok.status).toBe(200);
      expect(await ok.text()).toBe("hidden");
    } finally {
      await dropCollection(pb, name);
      await dropCollection(pb, users);
    }
  });
});
