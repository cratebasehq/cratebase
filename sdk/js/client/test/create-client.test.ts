import { afterEach, describe, expect, test } from "bun:test";
import { createClient, MemoryAuthStore } from "../src/index.js";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

interface Seen {
  url: string;
  method: string;
  auth: string | null;
  body: string | null;
}

function mockFetch(reply: unknown): Seen[] {
  const seen: Seen[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(input, init);
    seen.push({
      url: req.url,
      method: req.method,
      auth: req.headers.get("authorization"),
      body: init?.body ? String(init.body) : null,
    });
    return new Response(JSON.stringify(reply), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  return seen;
}

describe("createClient token", () => {
  test("token + authStore throws", () => {
    expect(() => createClient("http://x.test", { token: "cb_abc", authStore: new MemoryAuthStore() })).toThrow(
      "createClient: pass either `token` or `authStore`, not both",
    );
  });

  test("token is sent as a bearer header", async () => {
    const seen = mockFetch({ items: [], page: 1, perPage: 30, totalItems: 0, totalPages: 0 });
    const cb = createClient("http://x.test", { token: "cb_abc" });
    await cb.collection("posts").list();
    expect(seen[0]!.auth).toBe("Bearer cb_abc");
  });
});

describe("exportCookie", () => {
  function clientWithToken() {
    return createClient("http://x.test", { token: "tok123" });
  }

  test("default output is unchanged", () => {
    expect(clientWithToken().auth.exportCookie()).toBe("cb_session=tok123; Path=/; HttpOnly; SameSite=Lax");
  });

  test("secure + maxAge", () => {
    expect(clientWithToken().auth.exportCookie("cb_session", { secure: true, maxAge: 60 })).toBe(
      "cb_session=tok123; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age=60",
    );
  });

  test("SameSite=None requires secure", () => {
    const cb = clientWithToken();
    expect(() => cb.auth.exportCookie("cb_session", { sameSite: "None" })).toThrow("SameSite=None requires secure: true");
    expect(cb.auth.exportCookie("c", { sameSite: "None", secure: true, path: "/app" })).toBe(
      "c=tok123; Path=/app; HttpOnly; SameSite=None; Secure",
    );
  });
});

describe("llm usage", () => {
  test("logUsage posts the body", async () => {
    const seen = mockFetch({ id: "abc", costUsd: 0.00182 });
    const cb = createClient("http://x.test", { token: "cb_k" });
    const res = await cb.llm.logUsage({ model: "m1", promptTokens: 1000, completionTokens: 500, tokensCached: 200 });
    expect(res).toEqual({ id: "abc", costUsd: 0.00182 });
    expect(seen[0]!.method).toBe("POST");
    expect(seen[0]!.url).toBe("http://x.test/api/llm/usage");
    expect(JSON.parse(seen[0]!.body!)).toEqual({ model: "m1", promptTokens: 1000, completionTokens: 500, tokensCached: 200 });
  });

  test("usageSummary sends groupBy/from as query", async () => {
    const seen = mockFetch({ items: [] });
    const cb = createClient("http://x.test", { token: "cb_k" });
    await cb.llm.usageSummary({ groupBy: "day", from: "2026-01-01" });
    expect(seen[0]!.method).toBe("GET");
    const u = new URL(seen[0]!.url);
    expect(u.pathname).toBe("/api/llm/usage/summary");
    expect(u.searchParams.get("groupBy")).toBe("day");
    expect(u.searchParams.get("from")).toBe("2026-01-01");
    expect(u.searchParams.has("to")).toBe(false);
  });
});
