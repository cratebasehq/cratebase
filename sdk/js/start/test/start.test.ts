import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";

let request = new Request("http://app.test/");
const responseHeaders: Record<string, string> = {};

mock.module("@tanstack/react-start/server", () => ({
  getRequest: () => request,
  setResponseHeader: (name: string, value: string) => {
    responseHeaders[name] = value;
  },
}));

// A minimal stand-in for TanStack's builder: `.server(fn)` returns `fn` itself
// so the tests can invoke the middleware body directly.
mock.module("@tanstack/react-start", () => ({
  createMiddleware: () => ({ server: (fn: unknown) => fn }),
}));

// Dynamic import: the module mocks above must be registered before the module loads.
const { createCratebaseStart } = await import("../src/index.js");

const realFetch = globalThis.fetch;
let fetched: { url: string; auth: string | null }[] = [];

function jwt(expSeconds: number): string {
  const b64 = (o: unknown) => btoa(JSON.stringify(o)).replace(/=+$/, "").replace(/\+/g, "-").replace(/\//g, "_");
  return `${b64({ alg: "HS256" })}.${b64({ exp: expSeconds })}.sig`;
}

function setRequest(headers: Record<string, string>) {
  request = new Request("http://app.test/", { headers });
}

beforeEach(() => {
  fetched = [];
  for (const k of Object.keys(responseHeaders)) delete responseHeaders[k];
  setRequest({});
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(input, init);
    fetched.push({ url: req.url, auth: req.headers.get("authorization") });
    const body = { token: jwt(Math.floor(Date.now() / 1000) + 3600), record: { id: "u1", collectionName: "users" } };
    return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
});

afterEach(() => {
  globalThis.fetch = realFetch;
});

const URL_BASE = "http://cb.test";

describe("session cookie", () => {
  async function signIn() {
    const start = createCratebaseStart({ url: URL_BASE });
    await start.client().auth.signIn.password({ identity: "a@b.c", password: "pw" });
  }

  test("https via x-forwarded-proto sets Secure and Max-Age", async () => {
    setRequest({ "x-forwarded-proto": "https" });
    await signIn();
    expect(responseHeaders["Set-Cookie"]).toContain("Secure");
    expect(responseHeaders["Set-Cookie"]).toContain("Max-Age=2592000");
  });

  test("plain http has no Secure", async () => {
    await signIn();
    expect(responseHeaders["Set-Cookie"]).toContain("Max-Age=2592000");
    expect(responseHeaders["Set-Cookie"]).not.toContain("Secure");
  });
});

describe("getUser", () => {
  test("concurrent calls share one refresh", async () => {
    const start = createCratebaseStart({ url: URL_BASE });
    setRequest({ cookie: `cb_session=${jwt(Math.floor(Date.now() / 1000) + 3600)}` });
    const [a, b] = await Promise.all([start.getUser(), start.getUser()]);
    expect(a?.id).toBe("u1");
    expect(b?.id).toBe("u1");
    expect(fetched.filter((f) => f.url.endsWith("/auth-refresh"))).toHaveLength(1);
  });

  test("expired JWT returns null without a network call", async () => {
    const start = createCratebaseStart({ url: URL_BASE });
    setRequest({ cookie: `cb_session=${jwt(Math.floor(Date.now() / 1000) - 10)}` });
    expect(await start.getUser()).toBeNull();
    expect(fetched).toHaveLength(0);
  });
});

describe("adminClient", () => {
  test("sends the API key as a bearer token", async () => {
    const start = createCratebaseStart({ url: URL_BASE, apiKey: "cb_k" });
    await start.adminClient().collection("posts").list();
    expect(fetched[0]!.auth).toBe("Bearer cb_k");
  });

  test("throws without apiKey", () => {
    const start = createCratebaseStart({ url: URL_BASE });
    expect(() => start.adminClient()).toThrow("createCratebaseStart: apiKey is required for adminClient()");
  });
});

describe("authMiddleware", () => {
  test("throws a 401 Response when there is no cookie", async () => {
    const start = createCratebaseStart({ url: URL_BASE });
    const run = start.authMiddleware as unknown as (a: { next: (x: unknown) => unknown }) => Promise<unknown>;
    let thrown: unknown;
    try {
      await run({ next: () => "ok" });
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(Response);
    expect((thrown as Response).status).toBe(401);
  });
});
