import { beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { adminClient, client, expectError, uniq, waitFor } from "./harness";

/** Logs API: /api/logs (superuser only). Request logs are written asynchronously. */
let pb: PocketBase;

beforeAll(async () => {
  pb = await adminClient();
});

describe("logs", () => {
  test("a successful request produces a level 0 log row with the documented shape", async () => {
    const marker = uniq("logok");
    await pb.collections.create({ name: marker, type: "base", listRule: "", fields: [] });
    try {
      await fetch(`${pb.baseURL}/api/collections/${marker}/records`);
      const row = await waitFor(
        async () => (await pb.logs.getList(1, 1, { filter: `data.url ~ "${marker}"` })).items[0],
        { timeout: 10000, interval: 200 },
      );
      expect(row.id).toMatch(/^[a-z0-9]{15}$/);
      expect(row.created).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}Z$/);
      // NOTE: the SDK types LogModel.level as string, but the API returns a number
      expect(Number(row.level)).toBe(0);
      expect(row.message).toBe(`GET /api/collections/${marker}/records`);
      expect(Object.keys(row).sort()).toEqual(["created", "data", "id", "level", "message"]);
      const d = row.data as Record<string, unknown>;
      expect(d.type).toBe("request");
      expect(d.method).toBe("GET");
      expect(d.url).toBe(`/api/collections/${marker}/records`);
      expect(d.status).toBe(200);
      expect(typeof d.execTime).toBe("number");
      // `auth` is "" for guests unless settings.logs.logAuthId is on
      expect(d.auth).toBe("");
      expect(d.userIP).toBe("127.0.0.1");
      expect(d.remoteIP).toBe("127.0.0.1");
      expect(typeof d.referer).toBe("string");
      expect(typeof d.userAgent).toBe("string");
      expect(d).not.toHaveProperty("error");
    } finally {
      await pb.collections.delete(marker);
    }
  });

  test("a failed request is logged at level 8 with the error message", async () => {
    const marker = uniq("logerr");
    await fetch(`${pb.baseURL}/api/collections/${marker}/records`);
    const row = await waitFor(
      async () => (await pb.logs.getList(1, 1, { filter: `data.url ~ "${marker}"` })).items[0],
      { timeout: 10000, interval: 200 },
    );
    expect(Number(row.level)).toBe(8);
    const d = row.data as Record<string, unknown>;
    expect(d.status).toBe(404);
    expect(d.error).toBe("Missing collection context.");
  });

  test("getList pagination envelope + filter + sort", async () => {
    const page = await pb.logs.getList(1, 2, { sort: "-created" });
    expect(Object.keys(page).sort()).toEqual(["items", "page", "perPage", "totalItems", "totalPages"]);
    expect(page.page).toBe(1);
    expect(page.perPage).toBe(2);
    expect(page.totalItems).toBeGreaterThan(0);
    expect(page.items.length).toBeLessThanOrEqual(2);
    const filtered = await pb.logs.getList(1, 5, { filter: 'data.method = "GET"' });
    for (const it of filtered.items) expect((it.data as Record<string, unknown>).method).toBe("GET");
    const byLevel = await pb.logs.getList(1, 5, { filter: "level >= 0" });
    expect(byLevel.totalItems).toBeGreaterThan(0);
    const err = await expectError(() => pb.logs.getList(1, 1, { filter: "nonsense field" }));
    expect(err.status).toBe(400);
  });

  test("getOne by id / 404 for unknown", async () => {
    const first = (await pb.logs.getList(1, 1)).items[0]!;
    const one = await pb.logs.getOne(first.id);
    expect(one.id).toBe(first.id);
    expect(one.message).toBe(first.message);
    const err = await expectError(() => pb.logs.getOne("nonexistent0001"));
    expect(err.status).toBe(404);
    expect(err.response.message).toBe("The requested resource wasn't found.");
  });

  test("getStats returns hourly buckets", async () => {
    const stats = await pb.logs.getStats();
    expect(Array.isArray(stats)).toBe(true);
    expect(stats.length).toBeGreaterThan(0);
    for (const s of stats) {
      expect(Object.keys(s).sort()).toEqual(["date", "total"]);
      expect(s.date).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:00:00\.000Z$/);
      expect(typeof s.total).toBe("number");
    }
    const filtered = await pb.logs.getStats({ filter: 'data.method = "POST"' });
    expect(Array.isArray(filtered)).toBe(true);
  });

  test("logs are superuser-only", async () => {
    const err = await expectError(() => client().logs.getList());
    expect(err.status).toBe(401);
    expect(err.response.message).toBe("The request requires valid record authorization token.");
  });
});
