import { beforeEach, describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useRecords } from "../useRecords.js";
import { useRecord } from "../useRecord.js";
import { useInfiniteRecords } from "../useInfiniteRecords.js";
import { useChannel } from "../useChannel.js";
import { useMagicLinkCallback } from "../useMagicLinkCallback.js";
import { useUpload } from "../useUpload.js";
import { usePresence } from "../usePresence.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (v: T) => void;
}
function deferred<T>(): Deferred<T> {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe("useRecords", () => {
  test("an older, slower response never overwrites a newer one", async () => {
    const posts = createFakeCollection<any>([]);
    const pending: Array<Deferred<any[]>> = [];
    posts.fullList = () => {
      const d = deferred<any[]>();
      pending.push(d);
      return d.promise;
    };
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useRecords(client, "posts"));
    await waitFor(() => expect(pending).toHaveLength(1));
    await waitFor(() => expect(posts.listenerCount()).toBe(1));
    // A realtime event triggers a debounced refetch while request 1 is still in flight.
    act(() => posts.emit("*", { action: "create", record: { id: "x" } }));
    await waitFor(() => expect(pending).toHaveLength(2));

    // Resolve the newer request first, then the older one.
    await act(async () => {
      pending[1]!.resolve([{ id: "new" }]);
      await Promise.resolve();
    });
    await act(async () => {
      pending[0]!.resolve([{ id: "old" }]);
      await Promise.resolve();
    });
    expect(result.current.records.map((r) => r.id)).toEqual(["new"]);
  });

  test("enabled: false clears previously loaded records", async () => {
    const posts = createFakeCollection<any>([{ id: "a" }]);
    const client = createFakeClient({ posts });
    const { result, rerender } = renderHook(({ enabled }) => useRecords(client, "posts", { enabled, realtime: false }), {
      initialProps: { enabled: true },
    });
    await waitFor(() => expect(result.current.records).toHaveLength(1));
    rerender({ enabled: false });
    await waitFor(() => expect(result.current.records).toHaveLength(0));
    expect(result.current.loading).toBe(false);
  });

  test("a subscribe failure is not wiped by a successful refetch", async () => {
    const posts = createFakeCollection<any>([{ id: "a" }]);
    posts.subscribe = async () => {
      throw new Error("sub failed");
    };
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useRecords(client, "posts"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    await waitFor(() => expect((result.current.error as Error)?.message).toBe("sub failed"));
  });
});

describe("useRecord", () => {
  test("a realtime event applied before the fetch resolves is not overwritten by it", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "stale" }]);
    const d = deferred<any>();
    posts.one = () => d.promise;
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useRecord(client, "posts", "p1"));
    await waitFor(() => expect(posts.listenerCount()).toBe(1));
    act(() => posts.emit("p1", { action: "update", record: { id: "p1", title: "fresh" } }));
    await act(async () => {
      d.resolve({ id: "p1", title: "stale" });
      await Promise.resolve();
    });
    expect(result.current.record?.title).toBe("fresh");
  });

  test("deleted is cleared by a later non-delete event", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useRecord(client, "posts", "p1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => posts.emit("p1", { action: "delete", record: { id: "p1", title: "A" } }));
    expect(result.current.deleted).toBe(true);
    act(() => posts.emit("p1", { action: "create", record: { id: "p1", title: "back" } }));
    expect(result.current.deleted).toBe(false);
    expect(result.current.record?.title).toBe("back");
  });

  test("changing id clears the previous record", async () => {
    const posts = createFakeCollection<any>([
      { id: "p1", title: "A" },
      { id: "p2", title: "B" },
    ]);
    const d = deferred<any>();
    const realOne = posts.one;
    posts.one = (id: string) => (id === "p2" ? d.promise : realOne(id));
    const client = createFakeClient({ posts });
    const { result, rerender } = renderHook(({ id }) => useRecord(client, "posts", id), { initialProps: { id: "p1" } });
    await waitFor(() => expect(result.current.record?.title).toBe("A"));
    rerender({ id: "p2" });
    await waitFor(() => expect(result.current.record).toBeNull());
    await act(async () => {
      d.resolve({ id: "p2", title: "B" });
      await Promise.resolve();
    });
    await waitFor(() => expect(result.current.record?.title).toBe("B"));
  });
});

describe("useInfiniteRecords", () => {
  test("loadMore is a no-op while a fetch is in flight", async () => {
    const seed = Array.from({ length: 6 }, (_, i) => ({ id: `p${i}` }) as any);
    const posts = createFakeCollection<any>(seed);
    const calls: number[] = [];
    const realList = posts.list;
    const gate = deferred<void>();
    posts.list = async (o: any) => {
      calls.push(o.page);
      if (o.page === 2) await gate.promise;
      return realList(o);
    };
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useInfiniteRecords(client, "posts", { perPage: 2, sort: "id" }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => {
      result.current.loadMore();
      result.current.loadMore();
    });
    await waitFor(() => expect(result.current.loadingMore).toBe(true));
    act(() => result.current.loadMore());
    await act(async () => {
      gate.resolve();
      await Promise.resolve();
    });
    await waitFor(() => expect(result.current.records).toHaveLength(4));
    expect(calls).toEqual([1, 2]);
  });

  test("a query change resets to page 1 without fetching the old page number", async () => {
    const seed = Array.from({ length: 6 }, (_, i) => ({ id: `p${i}`, kind: i % 2 ? "a" : "b" }) as any);
    const posts = createFakeCollection<any>(seed);
    const pages: Array<[string | undefined, number]> = [];
    const realList = posts.list;
    posts.list = async (o: any) => {
      pages.push([o.filter, o.page]);
      return realList(o);
    };
    const client = createFakeClient({ posts });
    const { result, rerender } = renderHook(
      ({ filter }) => useInfiniteRecords(client, "posts", { perPage: 1, sort: "id", filter }),
      { initialProps: { filter: 'kind = "a"' as string } },
    );
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.records).toHaveLength(2));
    rerender({ filter: 'kind = "b"' });
    await waitFor(() => expect(result.current.records.every((r: any) => r.kind === "b")).toBe(true));
    expect(pages.filter(([f]) => f === 'kind = "b"').every(([, p]) => p === 1)).toBe(true);
  });
});

describe("useChannel", () => {
  test("connected drops on unmount and publish afterwards rejects", async () => {
    const client = createFakeClient();
    const { result, unmount } = renderHook(() => useChannel(client, "room"));
    await waitFor(() => expect(result.current.connected).toBe(true));
    const publish = result.current.publish;
    unmount();
    await expect(publish("x")).rejects.toThrow("closed");
  });

  test("connected resets when the channel name changes", async () => {
    const client = createFakeClient();
    const { result, rerender } = renderHook(({ name }) => useChannel(client, name), { initialProps: { name: "a" } });
    await waitFor(() => expect(result.current.connected).toBe(true));
    rerender({ name: "b" });
    await waitFor(() => expect(result.current.connected).toBe(true));
    expect(result.current.lastMessage).toBeNull();
  });
});

describe("useMagicLinkCallback", () => {
  beforeEach(() => window.history.pushState({}, "", "/callback"));

  test("a throwing onSuccess does not flip status to error", async () => {
    window.history.pushState({}, "", "/callback?token=good-token");
    const client = createFakeClient();
    const orig = console.error;
    console.error = () => {};
    try {
      const { result } = renderHook(() =>
        useMagicLinkCallback(client, {
          onSuccess: () => {
            throw new Error("boom");
          },
        }),
      );
      await waitFor(() => expect(result.current.status).toBe("success"));
      await new Promise((r) => setTimeout(r, 20));
      expect(result.current.status).toBe("success");
      expect(result.current.error).toBeNull();
    } finally {
      console.error = orig;
    }
  });
});

describe("useUpload", () => {
  test("overlapping uploads: pending until all settle; abort cancels all", async () => {
    const client = createFakeClient();
    const gates: Array<Deferred<void>> = [];
    (client as any).files.upload = async (file: any, options: any) => {
      const g = deferred<void>();
      gates.push(g);
      await new Promise<void>((resolve, reject) => {
        options.signal.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        g.promise.then(resolve);
      });
      return { token: "t", recordId: "r", filename: "f" };
    };
    const { result } = renderHook(() => useUpload(client));
    const opts = { collection: "posts", field: "cover" } as any;
    let p1!: Promise<unknown>, p2!: Promise<unknown>;
    act(() => {
      p1 = result.current.upload(new Blob(["a"]) as any, opts);
      p2 = result.current.upload(new Blob(["b"]) as any, opts);
    });
    await act(async () => {
      gates[0]!.resolve();
      await p1;
    });
    expect(result.current.pending).toBe(true);

    const rejected = p2.catch((e) => e);
    act(() => result.current.abort());
    expect(((await rejected) as Error).name).toBe("AbortError");
    await waitFor(() => expect(result.current.pending).toBe(false));
  });

  test("abort cancels every in-flight upload", async () => {
    const client = createFakeClient();
    (client as any).files.upload = (_f: any, options: any) =>
      new Promise((_, reject) => options.signal.addEventListener("abort", () => reject(new DOMException("x", "AbortError"))));
    const { result } = renderHook(() => useUpload(client));
    const opts = { collection: "posts", field: "cover" } as any;
    let p1!: Promise<unknown>, p2!: Promise<unknown>;
    act(() => {
      p1 = result.current.upload(new Blob(["a"]) as any, opts).catch((e) => e);
      p2 = result.current.upload(new Blob(["b"]) as any, opts).catch((e) => e);
    });
    act(() => result.current.abort());
    const [e1, e2] = await Promise.all([p1, p2]);
    expect((e1 as Error).name).toBe("AbortError");
    expect((e2 as Error).name).toBe("AbortError");
    await waitFor(() => expect(result.current.pending).toBe(false));
  });
});

describe("usePresence", () => {
  test("members from the previous channel are cleared on channel change", async () => {
    const client = createFakeClient();
    (client.channel("a") as any)._simulateJoin("peer", { x: 1 });
    const { result, rerender } = renderHook(({ ch }) => usePresence(client, ch, null), { initialProps: { ch: "a" } });
    await waitFor(() => expect(result.current.members).toHaveLength(1));
    rerender({ ch: "b" });
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.members).toHaveLength(0);
  });
});
