import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { adminClient, client, dropCollection, expectError, uniq } from "./harness";

/** Batch API: POST /api/batch (needs settings.batch.enabled). */
let pb: PocketBase;
const ITEMS = uniq("batchitems");

beforeAll(async () => {
  pb = await adminClient();
  await pb.collections.create({
    name: ITEMS,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: "",
    updateRule: "",
    deleteRule: "",
    fields: [
      { name: "title", type: "text", required: true, min: 3 },
      { name: "n", type: "number" },
      { name: "doc", type: "file", maxSelect: 1 },
    ],
  });

  await pb.settings.update({ batch: { enabled: true, maxRequests: 50, timeout: 3, maxBodySize: 0 } });
});

afterAll(async () => {
  await pb.settings.update({ batch: { enabled: false, maxRequests: 50, timeout: 3, maxBodySize: 0 } });
  await dropCollection(pb, ITEMS);
});

describe("batch", () => {
  test("create / update / upsert / delete in one request", async () => {
    const seed = await pb.collection(ITEMS).create({ title: "Seed", n: 1 });
    const doomed = await pb.collection(ITEMS).create({ title: "Doomed", n: 2 });
    const upsertId = "upsert" + Math.random().toString(36).slice(2, 11);

    const batch = pb.createBatch();
    batch.collection(ITEMS).create({ title: "Created in batch", n: 10 });
    batch.collection(ITEMS).update(seed.id, { n: 42 });
    batch.collection(ITEMS).upsert({ id: upsertId, title: "Upserted", n: 7 });
    batch.collection(ITEMS).delete(doomed.id);
    const res = await batch.send();

    expect(Array.isArray(res)).toBe(true);
    expect(res).toHaveLength(4);
    for (const r of res) expect(Object.keys(r).sort()).toEqual(["body", "status"]);
    expect(res.map((r) => r.status)).toEqual([200, 200, 200, 204]);
    expect(res[0]!.body.title).toBe("Created in batch");
    expect(res[0]!.body.collectionName).toBe(ITEMS);
    expect(res[0]!.body.id).toMatch(/^[a-z0-9]{15}$/);
    expect(res[1]!.body.n).toBe(42);
    expect(res[2]!.body.id).toBe(upsertId);
    expect(res[3]!.body).toBeNull();

    expect((await pb.collection(ITEMS).getOne(seed.id)).n).toBe(42);
    expect((await pb.collection(ITEMS).getOne(upsertId)).title).toBe("Upserted");
    const gone = await expectError(() => pb.collection(ITEMS).getOne(doomed.id));
    expect(gone.status).toBe(404);

    // upsert again updates the existing record
    const b2 = pb.createBatch();
    b2.collection(ITEMS).upsert({ id: upsertId, title: "Upserted twice", n: 8 });
    const res2 = await b2.send();
    expect(res2[0]!.body.title).toBe("Upserted twice");
    expect((await pb.collection(ITEMS).getList(1, 1, { filter: `id = "${upsertId}"` })).totalItems).toBe(1);
  });

  test("a failing request rolls the whole batch back", async () => {
    const before = (await pb.collection(ITEMS).getList(1, 1)).totalItems;
    const survivor = await pb.collection(ITEMS).create({ title: "Survivor", n: 1 });

    const batch = pb.createBatch();
    batch.collection(ITEMS).create({ title: "Fine", n: 1 });
    batch.collection(ITEMS).update(survivor.id, { n: 999 });
    batch.collection(ITEMS).create({ title: "x" }); // violates min=3
    const err = await expectError(() => batch.send());

    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Batch transaction failed.");
    // the failing request is reported under data.requests, keyed by its index
    const failed = err.response.data.requests["2"];
    expect(failed.code).toBe("batch_request_failed");
    expect(failed.message).toBe("Batch request failed.");
    expect(failed.response.status).toBe(400);
    expect(failed.response.message).toBe("Failed to create record.");
    expect(failed.response.data.title.code).toBe("validation_min_text_constraint");
    // successful requests are not echoed back
    expect(Object.keys(err.response.data.requests)).toEqual(["2"]);

    // nothing was applied
    expect((await pb.collection(ITEMS).getList(1, 1)).totalItems).toBe(before + 1);
    expect((await pb.collection(ITEMS).getOne(survivor.id)).n).toBe(1);
    await pb.collection(ITEMS).delete(survivor.id);
  });

  test("batch honours API rules of the executing client", async () => {
    const name = uniq("batchlocked");
    await pb.collections.create({ name, type: "base", fields: [{ name: "title", type: "text" }] });
    try {
      const anon = client();
      const batch = anon.createBatch();
      batch.collection(name).create({ title: "nope" });
      const err = await expectError(() => batch.send());
      expect(err.status).toBe(400);
      expect(err.response.message).toBe("Batch transaction failed.");
      expect(err.response.data.requests["0"].response.status).toBe(403);
      expect(err.response.data.requests["0"].response.message).toBe("Only superusers can perform this action.");
      expect((await pb.collection(name).getList(1, 1)).totalItems).toBe(0);
    } finally {
      await dropCollection(pb, name);
    }
  });

  test("file uploads inside a batch (multipart @jsonPayload)", async () => {
    const batch = pb.createBatch();
    batch.collection(ITEMS).create({ title: "With file", doc: new File(["batched"], "b.txt", { type: "text/plain" }) });
    const res = await batch.send();
    expect(res[0]!.status).toBe(200);
    const rec = res[0]!.body;
    expect(rec.doc).toMatch(/^b[a-z0-9_]*_[a-z0-9]{10}\.txt$/);
    const file = await fetch(pb.files.getURL(rec, rec.doc));
    expect(await file.text()).toBe("batched");
    await pb.collection(ITEMS).delete(rec.id);
  });

  test("exceeding maxRequests -> 400", async () => {
    await pb.settings.update({ batch: { enabled: true, maxRequests: 2, timeout: 3, maxBodySize: 0 } });
    try {
      const batch = pb.createBatch();
      for (let i = 0; i < 3; i++) batch.collection(ITEMS).create({ title: `Too many ${i}` });
      const err = await expectError(() => batch.send());
      expect(err.status).toBe(400);
      expect(err.response.data.requests.code).toBe("validation_length_too_long");
    } finally {
      await pb.settings.update({ batch: { enabled: true, maxRequests: 50, timeout: 3, maxBodySize: 0 } });
    }
  });

  test("batch disabled -> 403", async () => {
    await pb.settings.update({ batch: { enabled: false, maxRequests: 50, timeout: 3, maxBodySize: 0 } });
    try {
      const batch = pb.createBatch();
      batch.collection(ITEMS).create({ title: "Disabled" });
      const err = await expectError(() => batch.send());
      expect(err.status).toBe(403);
      expect(err.response.message).toBe("Batch requests are not allowed.");
    } finally {
      await pb.settings.update({ batch: { enabled: true, maxRequests: 50, timeout: 3, maxBodySize: 0 } });
    }
  });

  test("empty batch -> 400", async () => {
    const err = await expectError(() => pb.createBatch().send());
    expect(err.status).toBe(400);
    expect(err.response.data.requests.code).toBe("validation_required");
  });
});
