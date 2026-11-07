import { afterEach, describe, expect, test } from "bun:test";
import { createClient, MemoryAuthStore } from "../src/index.js";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

const enc = new TextEncoder();

interface Stream {
  auth: string | null;
  push(event: string, data: unknown): void;
  close(): void;
}

interface Harness {
  streams: Stream[];
  posts: Array<{ clientId: string; subscriptions: string[] }>;
  /** Next N stream GETs fail with 500. */
  failStreams: number;
  /** Next stream GET closes before CB_CONNECT. */
  closeBeforeConnect: boolean;
  postDelay: (n: number) => number;
  inFlightPosts: number;
  maxInFlightPosts: number;
}

function install(): Harness {
  const h: Harness = {
    streams: [],
    posts: [],
    failStreams: 0,
    closeBeforeConnect: false,
    postDelay: () => 0,
    inFlightPosts: 0,
    maxInFlightPosts: 0,
  };
  let postCount = 0;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(input, init);
    if (req.method === "GET") {
      if (h.failStreams > 0) {
        h.failStreams--;
        return new Response("boom", { status: 500 });
      }
      let controller!: ReadableStreamDefaultController<Uint8Array>;
      const body = new ReadableStream<Uint8Array>({
        start(c) {
          controller = c;
        },
      });
      const id = `c${h.streams.length + 1}`;
      const stream: Stream = {
        auth: req.headers.get("authorization"),
        push: (event, data) => controller.enqueue(enc.encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`)),
        close: () => controller.close(),
      };
      h.streams.push(stream);
      init?.signal?.addEventListener("abort", () => {
        try {
          controller.close();
        } catch {
          // already closed
        }
      });
      if (h.closeBeforeConnect) {
        h.closeBeforeConnect = false;
        controller.close();
      } else {
        stream.push("CB_CONNECT", { clientId: id });
      }
      return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
    }
    const n = postCount++;
    h.inFlightPosts++;
    h.maxInFlightPosts = Math.max(h.maxInFlightPosts, h.inFlightPosts);
    const parsed = JSON.parse(String(init?.body)) as { clientId: string; subscriptions: string[] };
    await new Promise((r) => setTimeout(r, h.postDelay(n)));
    h.posts.push(parsed);
    h.inFlightPosts--;
    return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  return h;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("RealtimeClient state machine", () => {
  test("non-ok stream response is not cached; later subscribe retries", async () => {
    const h = install();
    h.failStreams = 1;
    const cb = createClient("http://x.test");
    await expect(cb.realtime.subscribeTopic("a", () => {})).rejects.toThrow("status 500");
    await cb.realtime.subscribeTopic("a", () => {});
    expect(h.streams.length).toBe(1);
    await cb.realtime.stop();
  });

  test("stream closing before CB_CONNECT rejects instead of hanging", async () => {
    const h = install();
    h.closeBeforeConnect = true;
    const cb = createClient("http://x.test");
    await expect(cb.realtime.subscribeTopic("a", () => {})).rejects.toThrow("closed before CB_CONNECT");
    await cb.realtime.subscribeTopic("a", () => {});
    await cb.realtime.stop();
  });

  test("calling an unsubscribe twice does not drop a newer listener set", async () => {
    const h = install();
    const cb = createClient("http://x.test");
    const got: unknown[] = [];
    const unsubA = await cb.realtime.subscribeTopic("k", () => {});
    await unsubA();
    await cb.realtime.subscribeTopic("k", (e) => got.push(e));
    await unsubA();
    h.streams[0]!.push("k", { n: 1 });
    await sleep(20);
    expect(got).toEqual([{ n: 1 }]);
    await cb.realtime.stop();
  });

  test("subscription POSTs are serialised and the latest state wins", async () => {
    const h = install();
    h.postDelay = (n) => (n === 0 ? 30 : 0);
    const cb = createClient("http://x.test");
    await Promise.all([
      cb.realtime.subscribeTopic("a", () => {}),
      cb.realtime.subscribeTopic("b", () => {}),
      cb.realtime.subscribeTopic("c", () => {}),
    ]);
    expect(h.maxInFlightPosts).toBe(1);
    expect(h.posts[h.posts.length - 1]!.subscriptions.sort()).toEqual(["a", "b", "c"]);
    await cb.realtime.stop();
  });

  test("stop() then subscribe starts a fresh connection", async () => {
    const h = install();
    const cb = createClient("http://x.test");
    await cb.realtime.subscribeTopic("a", () => {});
    await cb.realtime.stop();
    await cb.realtime.subscribeTopic("a", () => {});
    expect(h.streams.length).toBe(2);
    expect(h.posts[h.posts.length - 1]!.clientId).toBe("c2");
    await cb.realtime.stop();
  });

  test("auth change reconnects with the new token; failed reconnect backs off and retries", async () => {
    const h = install();
    const store = new MemoryAuthStore();
    const cb = createClient("http://x.test", { authStore: store });
    await cb.realtime.subscribeTopic("a", () => {});
    expect(h.streams[0]!.auth).toBeNull();

    h.failStreams = 1; // first reconnect attempt fails; must reschedule
    store.save("tok", null);
    await sleep(900);
    expect(h.streams.length).toBe(2);
    expect(h.streams[1]!.auth).toBe("Bearer tok");
    const last = h.posts[h.posts.length - 1]!;
    expect(last.clientId).toBe("c2");
    expect(last.subscriptions).toEqual(["a"]);
    await cb.realtime.stop();
  });
});
