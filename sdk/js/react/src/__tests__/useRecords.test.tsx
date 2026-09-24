import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useRecords } from "../useRecords.js";
import { CratebaseProvider } from "../context.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

interface Post {
  id: string;
  title: string;
  published: boolean;
}

describe("useRecords", () => {
  test("fetches the full list on mount (explicit client)", async () => {
    const posts = createFakeCollection<Post & any>([
      { id: "p1", title: "A", published: true } as any,
      { id: "p2", title: "B", published: false } as any,
    ]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecords(client, "posts"));

    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.records.map((r: any) => r.id).sort()).toEqual(["p1", "p2"]);
    expect(result.current.error).toBeNull();
  });

  test("resolves the client from CratebaseProvider context when omitted", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A", published: true }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecords("posts"), {
      wrapper: ({ children }) => <CratebaseProvider client={client}>{children}</CratebaseProvider>,
    });

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.records).toHaveLength(1);
  });

  test("throws when no client is available (no provider, no explicit client)", () => {
    // Swallow the expected React error-boundary console noise for this
    // one assertion.
    const originalError = console.error;
    console.error = () => {};
    try {
      expect(() => renderHook(() => useRecords("posts"))).toThrow(/CratebaseProvider/);
    } finally {
      console.error = originalError;
    }
  });

  test("paginates via list() when page/perPage are given", async () => {
    const seed = Array.from({ length: 5 }, (_, i) => ({ id: `p${i}`, title: `t${i}`, published: true }) as any);
    const posts = createFakeCollection<any>(seed);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecords(client, "posts", { page: 1, perPage: 2, sort: "title" }));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.records).toHaveLength(2);
    expect(result.current.totalItems).toBe(5);
    expect(result.current.totalPages).toBe(3);
    expect(result.current.page).toBe(1);
  });

  test("realtime: true (default) refetches after a matching event", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A", published: true }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecords(client, "posts"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.records).toHaveLength(1);

    await act(async () => {
      await posts.create({ title: "B", published: true });
    });

    await waitFor(() => expect(result.current.records).toHaveLength(2));
  });

  test("realtime: false does not subscribe", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A", published: true }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecords(client, "posts", { realtime: false }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(posts.listenerCount()).toBe(0);
  });

  test("enabled: false skips fetching entirely", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A", published: true }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecords(client, "posts", { enabled: false }));
    expect(result.current.loading).toBe(false);
    expect(result.current.records).toEqual([]);
  });

  test("refresh() re-runs the fetch on demand", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A", published: true }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecords(client, "posts", { realtime: false }));
    await waitFor(() => expect(result.current.loading).toBe(false));

    // Mutate outside the hook's own write path (no realtime subscribed),
    // so only an explicit refresh() should pick it up.
    await act(async () => {
      await posts.create({ title: "B", published: true });
    });
    expect(result.current.records).toHaveLength(1);

    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.records).toHaveLength(2));
  });

  test("unsubscribes on unmount", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A", published: true }]);
    const client = createFakeClient({ posts });

    const { result, unmount } = renderHook(() => useRecords(client, "posts"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(posts.listenerCount()).toBeGreaterThan(0);

    unmount();
    expect(posts.listenerCount()).toBe(0);
  });
});
