import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useRecord } from "../useRecord.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

describe("useRecord", () => {
  test("fetches one record by id", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecord(client, "posts", "p1"));
    expect(result.current.loading).toBe(true);

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.record?.title).toBe("A");
    expect(result.current.deleted).toBe(false);
  });

  test("applies its own update event directly (no refetch)", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecord(client, "posts", "p1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await posts.update("p1", { title: "B" });
    });

    await waitFor(() => expect(result.current.record?.title).toBe("B"));
  });

  test("marks deleted: true on a delete event, keeping the last value", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecord(client, "posts", "p1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await posts.delete("p1");
    });

    await waitFor(() => expect(result.current.deleted).toBe(true));
    expect(result.current.record?.title).toBe("A");
  });

  test("id: null/undefined or enabled: false skips fetching", () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });

    const { result } = renderHook(() => useRecord(client, "posts", null));
    expect(result.current.loading).toBe(false);
    expect(result.current.record).toBeNull();
  });
});
