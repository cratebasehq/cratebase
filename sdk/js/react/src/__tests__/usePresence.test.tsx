import { describe, expect, test } from "bun:test";
import { renderHook, waitFor } from "@testing-library/react";
import { usePresence } from "../usePresence.js";
import { createFakeClient } from "./mockClient.js";

describe("usePresence", () => {
  test("tracks this client's own state and lists it as a member", async () => {
    const client = createFakeClient();
    const { result } = renderHook(() => usePresence(client, "room1", { name: "alice" }));

    await waitFor(() => expect(result.current.loading).toBe(false));
    const channel = client.channel("room1");
    await waitFor(() =>
      expect(result.current.members.some((m) => m.clientId === channel.selfClientId)).toBe(true),
    );
    const self = result.current.members.find((m) => m.clientId === channel.selfClientId);
    expect(self?.state).toEqual({ name: "alice" });
  });

  test("sees another peer's join/leave via presence.onChange", async () => {
    const client = createFakeClient();
    const { result } = renderHook(() => usePresence(client, "room1", null));
    await waitFor(() => expect(result.current.loading).toBe(false));

    const channel = client.channel("room1");
    channel._simulateJoin("peer1", { name: "bob" });
    await waitFor(() => expect(result.current.members.length).toBe(1));
    expect(result.current.members[0]).toEqual({ clientId: "peer1", state: { name: "bob" }, auth: null });

    channel._simulateLeave("peer1");
    await waitFor(() => expect(result.current.members.length).toBe(0));
  });

  test("onChange callback fires with the event kind", async () => {
    const client = createFakeClient();
    const seen: string[] = [];
    const { result } = renderHook(() =>
      usePresence(client, "room1", null, { onChange: (kind) => seen.push(kind) }),
    );
    await waitFor(() => expect(result.current.loading).toBe(false));

    const channel = client.channel("room1");
    channel._simulateJoin("peer1", {});
    channel._simulateJoin("peer1", { updated: true });
    channel._simulateLeave("peer1");
    await waitFor(() => expect(seen).toEqual(["join", "update", "leave"]));
  });

  test("state: null never tracks (no self member appears)", async () => {
    const client = createFakeClient();
    const { result } = renderHook(() => usePresence(client, "room1", null));
    await waitFor(() => expect(result.current.loading).toBe(false));

    const channel = client.channel("room1");
    await new Promise((r) => setTimeout(r, 10));
    expect(result.current.members.some((m) => m.clientId === channel.selfClientId)).toBe(false);
  });
});
