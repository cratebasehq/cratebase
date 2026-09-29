import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useNotifications } from "../useNotifications.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

function seedNotification(overrides: Record<string, unknown> = {}) {
  return {
    id: `n_${Math.random().toString(36).slice(2)}`,
    collectionId: "c1",
    collectionName: "_notifications",
    type: "info",
    title: "Hello",
    body: "World",
    data: {},
    link: "",
    readAt: "",
    created: new Date().toISOString(),
    ...overrides,
  };
}

describe("useNotifications", () => {
  test("fetches items and the unread count", async () => {
    const notifications = createFakeCollection([
      seedNotification({ id: "n1" }),
      seedNotification({ id: "n2", readAt: new Date().toISOString() }),
    ]);
    const client = createFakeClient({ _notifications: notifications });

    const { result } = renderHook(() => useNotifications(client));
    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.items).toHaveLength(2);
    expect(result.current.unreadCount).toBe(1);
    expect(result.current.error).toBeNull();
  });

  test("markRead marks one notification read and refreshes the count", async () => {
    const notifications = createFakeCollection([seedNotification({ id: "n1" })]);
    const client = createFakeClient({ _notifications: notifications });

    const { result } = renderHook(() => useNotifications(client));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.unreadCount).toBe(1);

    await act(async () => {
      await result.current.markRead("n1");
    });

    await waitFor(() => expect(result.current.unreadCount).toBe(0));
    expect(result.current.items.find((n) => n.id === "n1")?.readAt).not.toBe("");
  });

  test("markAllRead clears every unread notification", async () => {
    const notifications = createFakeCollection([
      seedNotification({ id: "n1" }),
      seedNotification({ id: "n2" }),
    ]);
    const client = createFakeClient({ _notifications: notifications });

    const { result } = renderHook(() => useNotifications(client));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.unreadCount).toBe(2);

    await act(async () => {
      await result.current.markAllRead();
    });

    await waitFor(() => expect(result.current.unreadCount).toBe(0));
  });

  test("refetches on a realtime event for this recipient", async () => {
    const notifications = createFakeCollection([seedNotification({ id: "n1" })]);
    const client = createFakeClient({ _notifications: notifications });

    const { result } = renderHook(() => useNotifications(client));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.unreadCount).toBe(1);

    await act(async () => {
      await notifications.create(seedNotification({ id: "n2" }));
    });

    await waitFor(() => expect(result.current.unreadCount).toBe(2));
  });

  test("enabled: false skips fetching entirely", () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useNotifications(client, { enabled: false }));
    expect(result.current.loading).toBe(false);
    expect(result.current.items).toHaveLength(0);
  });

  test("accepts an options object with no explicit client outside a provider", () => {
    expect(() => renderHook(() => useNotifications({ enabled: false }))).toThrow();
  });
});
