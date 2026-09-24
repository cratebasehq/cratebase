import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useSubscription } from "../useSubscription.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

describe("useSubscription", () => {
  test("calls back with raw events on the given topic (explicit client)", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });
    const events: any[] = [];

    renderHook(() => useSubscription(client, "posts", "*", (e) => events.push(e)));
    await waitFor(() => expect(posts.listenerCount()).toBeGreaterThan(0));

    await act(async () => {
      await posts.create({ title: "B" });
    });

    await waitFor(() => expect(events).toHaveLength(1));
    expect(events[0].action).toBe("create");
    expect(events[0].record.title).toBe("B");
  });

  test("unsubscribes on unmount", async () => {
    const posts = createFakeCollection<any>([]);
    const client = createFakeClient({ posts });

    const { unmount } = renderHook(() => useSubscription(client, "posts", "*", () => {}));
    await waitFor(() => expect(posts.listenerCount()).toBe(1));

    unmount();
    expect(posts.listenerCount()).toBe(0);
  });
});
