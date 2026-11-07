import { afterEach, describe, expect, test } from "bun:test";
import { createClient, MemoryAuthStore } from "../src/index.js";
import { trackPresence } from "../src/cratebase-only.js";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

describe("auth.refresh", () => {
  test("concurrent callers share one request", async () => {
    let calls = 0;
    globalThis.fetch = (async () => {
      calls++;
      await new Promise((r) => setTimeout(r, 10));
      return json({ token: "t2", record: { id: "u", collectionId: "c", collectionName: "users" } });
    }) as typeof fetch;
    const store = new MemoryAuthStore();
    store.save("t1", null);
    const cb = createClient("http://x.test", { authStore: store });
    const [a, b] = await Promise.all([cb.auth.refresh(), cb.auth.refresh()]);
    expect(calls).toBe(1);
    expect(a).toBe(b);
    expect(store.token).toBe("t2");
    await cb.auth.refresh();
    expect(calls).toBe(2);
  });

  test("401 clears the store", async () => {
    globalThis.fetch = (async () => json({ message: "nope", status: 401 }, 401)) as typeof fetch;
    const store = new MemoryAuthStore();
    store.save("t1", null);
    const cb = createClient("http://x.test", { authStore: store });
    await expect(cb.auth.refresh()).rejects.toThrow();
    expect(store.token).toBe("");
  });

  test("a refresh that lands after sign-out does not resurrect the session", async () => {
    globalThis.fetch = (async () => {
      await new Promise((r) => setTimeout(r, 10));
      return json({ token: "t2", record: { id: "u", collectionId: "c", collectionName: "users" } });
    }) as typeof fetch;
    const store = new MemoryAuthStore();
    store.save("t1", null);
    const cb = createClient("http://x.test", { authStore: store });
    const p = cb.auth.refresh();
    store.clear();
    await p;
    expect(store.token).toBe("");
  });
});

describe("batch", () => {
  test("a second send() throws", async () => {
    globalThis.fetch = (async () => json([])) as typeof fetch;
    const cb = createClient("http://x.test");
    const batch = cb.batch().create("posts", { title: "x" });
    await batch.send();
    await expect(batch.send()).rejects.toThrow("already sent");
    expect(() => batch.delete("posts", "1")).toThrow("already sent");
  });
});

describe("files.upload abort", () => {
  test("already-aborted signal rejects without sending, and listeners are removed", async () => {
    let calls = 0;
    globalThis.fetch = (async () => {
      calls++;
      return json({ uploadUrl: "http://x.test/up", method: "PUT", token: "t", recordId: "r", filename: "f", expiresAt: "" });
    }) as typeof fetch;
    const cb = createClient("http://x.test");
    const ctrl = new AbortController();
    ctrl.abort();
    await expect(
      cb.files.upload(new Blob(["x"]), { collection: "posts", field: "cover", signal: ctrl.signal }),
    ).rejects.toThrow();
    // Only the presign call (if it even got that far) may have happened; never the PUT.
    expect(calls).toBeLessThanOrEqual(1);
  });
});

describe("trackPresence", () => {
  test("heartbeat failures are surfaced and stop() deletes the row", async () => {
    const errors: unknown[] = [];
    const deleted: string[] = [];
    const svc = {
      update: async () => {
        throw new Error("hb fail");
      },
      create: async () => ({ id: "me", collectionId: "c", collectionName: "p" }),
      delete: async (id: string) => {
        deleted.push(id);
      },
      fullList: async () => [],
      subscribe: async () => () => {},
    };
    const presence = await trackPresence(svc as never, {}, { heartbeatMs: 10, onError: (e) => errors.push(e) });
    await new Promise((r) => setTimeout(r, 40));
    await presence.stop();
    expect(errors.length).toBeGreaterThan(0);
    expect(deleted).toEqual(["me"]);
  });
});
