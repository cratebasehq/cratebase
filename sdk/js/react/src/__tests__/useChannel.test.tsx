import { describe, expect, test } from "bun:test";
import { renderHook, waitFor } from "@testing-library/react";
import { useChannel } from "../useChannel.js";
import { createFakeClient } from "./mockClient.js";

describe("useChannel", () => {
  test("connects and receives a published message", async () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useChannel(client, "room1"));

    await waitFor(() => expect(result.current.connected).toBe(true));
    expect(result.current.lastMessage).toBeNull();

    await result.current.publish("chat", { text: "hi" });
    await waitFor(() => expect(result.current.lastMessage).not.toBeNull());
    expect(result.current.lastMessage).toEqual({ event: "chat", data: { text: "hi" } });
  });

  test("onMessage fires for every message, including ones after the first", async () => {
    const client = createFakeClient();
    const received: unknown[] = [];
    const { result } = renderHook(() =>
      useChannel(client, "room1", { onMessage: (m) => received.push(m) }),
    );
    await waitFor(() => expect(result.current.connected).toBe(true));

    await result.current.publish("a", 1);
    await result.current.publish("b", 2);
    await waitFor(() => expect(received.length).toBe(2));
    expect(received).toEqual([
      { event: "a", data: 1 },
      { event: "b", data: 2 },
    ]);
  });

  test("unsubscribes on unmount", async () => {
    const client = createFakeClient();
    const { result, unmount } = renderHook(() => useChannel(client, "room1"));
    await waitFor(() => expect(result.current.connected).toBe(true));

    const channel = client.channel("room1");
    expect(channel.listenerCount().messages).toBe(1);
    unmount();
    expect(channel.listenerCount().messages).toBe(0);
  });

  test("enabled: false skips subscribing", () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useChannel(client, "room1", { enabled: false }));
    expect(result.current.connected).toBe(false);
    const channel = client.channel("room1");
    expect(channel.listenerCount().messages).toBe(0);
  });
});
