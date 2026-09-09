import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { createClient, CratebaseError, filter, raw, type CratebaseClient } from "@cratebase/client";
import { ADMIN_EMAIL, ADMIN_PASSWORD, BASE_URL, uniq, waitFor } from "./harness";

/**
 * `@cratebase/client`'s own surface — typed CRUD, the `filter`/`raw` tags,
 * realtime, files, and batch. Every other file in this directory drives the
 * official `pocketbase` SDK to prove wire compatibility; this one (and
 * `client-auth.test.ts`) proves the first-party SDK actually works against
 * a live server, including setup, which goes through the SDK under test
 * itself (`cb.admin.collections`) rather than a second client.
 */

async function expectCbError(fn: () => Promise<unknown>): Promise<CratebaseError> {
  try {
    await fn();
  } catch (e) {
    if (e instanceof CratebaseError) return e;
    throw new Error(`expected CratebaseError, got ${String(e)}`);
  }
  throw new Error("expected the call to throw, but it resolved");
}

const PNG = Uint8Array.from(
  atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="),
  (c) => c.charCodeAt(0),
);
const png = (name = "pixel.png") => new File([PNG], name, { type: "image/png" });

let cb: CratebaseClient;
const ITEMS = uniq("clientitems");
const DOCS = uniq("clientdocs");

beforeAll(async () => {
  cb = createClient(BASE_URL, { authCollection: "_superusers" });
  await cb.auth.signIn.password({ identity: ADMIN_EMAIL, password: ADMIN_PASSWORD });
  await cb.admin.collections.create({
    name: ITEMS,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: "",
    updateRule: "",
    deleteRule: "",
    fields: [
      { name: "title", type: "text", required: true, min: 1 },
      { name: "n", type: "number" },
    ],
  });
  await cb.admin.collections.create({
    name: DOCS,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: "",
    updateRule: "",
    deleteRule: "",
    fields: [
      { name: "title", type: "text" },
      { name: "doc", type: "file", maxSelect: 1, maxSize: 5242880, mimeTypes: ["image/png"], thumbs: ["50x50"] },
    ],
  });
});

afterAll(async () => {
  await cb.admin.collections.delete(ITEMS).catch(() => {});
  await cb.admin.collections.delete(DOCS).catch(() => {});
});

describe("client: records", () => {
  test("create / one / update / delete round trip", async () => {
    const created = await cb.collection(ITEMS).create({ title: "Hello", n: 1 });
    expect(created.id).toBeTruthy();
    expect(created.title).toBe("Hello");

    const fetched = await cb.collection(ITEMS).one(created.id);
    expect(fetched.title).toBe("Hello");

    const updated = await cb.collection(ITEMS).update(created.id, { title: "Updated" });
    expect(updated.title).toBe("Updated");

    await cb.collection(ITEMS).delete(created.id);
    const err = await expectCbError(() => cb.collection(ITEMS).one(created.id));
    expect(err.status).toBe(404);
  });

  test("list() honours the filter/raw tags", async () => {
    const a = await cb.collection(ITEMS).create({ title: "Alpha", n: 5 });
    const b = await cb.collection(ITEMS).create({ title: "Beta", n: 5 });
    try {
      const res = await cb
        .collection(ITEMS)
        .list({ filter: filter`n = ${5} && ${raw("title")} ~ ${"Alph"}` });
      expect(res.items.map((r) => r.id)).toEqual([a.id]);
    } finally {
      await cb.collection(ITEMS).delete(a.id);
      await cb.collection(ITEMS).delete(b.id);
    }
  });

  test("list({ skipTotal: true }) -> totalItems === -1", async () => {
    const res = await cb.collection(ITEMS).list({ perPage: 1, skipTotal: true });
    expect(res.totalItems).toBe(-1);
    expect(res.totalPages).toBe(-1);
  });

  test("fullList paginates transparently", async () => {
    const marker = filter`n = ${9}`;
    const rows = await Promise.all(
      Array.from({ length: 3 }, (_, i) => cb.collection(ITEMS).create({ title: `Full ${i}`, n: 9 })),
    );
    try {
      const all = await cb.collection(ITEMS).fullList({ filter: marker });
      expect(all.length).toBe(3);
    } finally {
      await Promise.all(rows.map((r) => cb.collection(ITEMS).delete(r.id)));
    }
  });
});

describe("client: realtime", () => {
  test("subscribe receives a create event", async () => {
    const events: Array<{ action: string; record: { id: string } }> = [];
    const unsubscribe = await cb.collection(ITEMS).subscribe("*", (event) => events.push(event as (typeof events)[number]));
    try {
      const created = await cb.collection(ITEMS).create({ title: "Live", n: 1 });
      await waitFor(() => events.length > 0);
      expect(events[0]!.action).toBe("create");
      expect(events[0]!.record.id).toBe(created.id);
      await cb.collection(ITEMS).delete(created.id);
    } finally {
      unsubscribe();
    }
  });
});

describe("client: files", () => {
  test("files.url builds a fetchable URL, thumb query included", async () => {
    const form = new FormData();
    form.append("title", "Pixel");
    form.append("doc", png());
    const created = await cb.collection(DOCS).create(form);
    try {
      const url = cb.files.url(created, created.doc as string, { thumb: "50x50" });
      expect(url).toContain(`/api/files/${created.collectionId}/${created.id}/${created.doc}`);
      expect(url).toContain("thumb=50x50");
      const res = await fetch(url);
      expect(res.status).toBe(200);
      expect(res.headers.get("content-type")).toBe("image/png");
    } finally {
      await cb.collection(DOCS).delete(created.id);
    }
  });
});

describe("client: batch", () => {
  test("a failing sub-request is reported by index, whole batch rolled back", async () => {
    const before = (await cb.collection(ITEMS).list({ perPage: 1, skipTotal: true })).items.length;
    const survivor = await cb.collection(ITEMS).create({ title: "Survivor", n: 1 });
    await cb.admin.settings.update({ batch: { enabled: true, maxRequests: 50, timeout: 3, maxBodySize: 0 } });
    try {
      const batch = cb.batch();
      batch.create(ITEMS, { title: "Fine", n: 1 });
      batch.update(ITEMS, survivor.id, { n: 999 });
      batch.create(ITEMS, {}); // missing required `title` -> 400
      const err = await expectCbError(() => batch.send());
      expect(err.status).toBe(400);
      const requests = err.response.data.requests as unknown as Record<string, { response: { status: number } }>;
      expect(requests["2"]!.response.status).toBe(400);

      const stillSurvivor = await cb.collection(ITEMS).one(survivor.id);
      expect(stillSurvivor.n).toBe(1);
      expect((await cb.collection(ITEMS).list({ perPage: 1, skipTotal: true })).items.length).toBe(before + 1);
    } finally {
      await cb.collection(ITEMS).delete(survivor.id);
      await cb.admin.settings.update({ batch: { enabled: false, maxRequests: 50, timeout: 3, maxBodySize: 0 } });
    }
  });
});
