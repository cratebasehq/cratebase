import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useInfiniteRecords } from "../useInfiniteRecords.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

describe("useInfiniteRecords", () => {
  test("loads the first page, then appends on loadMore()", async () => {
    const seed = Array.from({ length: 5 }, (_, i) => ({ id: `p${i}`, title: `t${i}` }) as any);
    const posts = createFakeCollection<any>(seed);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useInfiniteRecords(client, "posts", { perPage: 2, sort: "id" }));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.records).toHaveLength(2);
    expect(result.current.hasMore).toBe(true);

    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.records).toHaveLength(4));
    expect(result.current.hasMore).toBe(true);

    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.records).toHaveLength(5));
    expect(result.current.hasMore).toBe(false);
  });

  test("refresh() discards loaded pages and refetches from page 1", async () => {
    const seed = Array.from({ length: 4 }, (_, i) => ({ id: `p${i}`, title: `t${i}` }) as any);
    const posts = createFakeCollection<any>(seed);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useInfiniteRecords(client, "posts", { perPage: 2, sort: "id" }));
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.records).toHaveLength(4));

    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.records).toHaveLength(2));
  });

  test("does not subscribe to realtime by default", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useInfiniteRecords(client, "posts"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(posts.listenerCount()).toBe(0);
  });

  test("realtime: true refreshes on a matching event", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useInfiniteRecords(client, "posts", { realtime: true }));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await posts.create({ title: "B" });
    });

    await waitFor(() => expect(result.current.records).toHaveLength(2));
  });
});
