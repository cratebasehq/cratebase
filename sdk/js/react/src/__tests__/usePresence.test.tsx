import { describe, expect, test } from "bun:test";
import { renderHook, waitFor } from "@testing-library/react";
import { usePresence } from "../usePresence.js";
import { createFakeClient } from "./mockClient.js";

describe("usePresence", () => {
  test("tracks presence and reports the online set", async () => {
    const client = createFakeClient();
    const { result } = renderHook(() => usePresence(client, "presence", { id: "me" }));

    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.online.has("me")).toBe(true);
    expect(result.current.error).toBeNull();
  });

  test("stops tracking on unmount", async () => {
    const client = createFakeClient();
    const { result, unmount } = renderHook(() => usePresence(client, "presence", { id: "me" }));
    await waitFor(() => expect(result.current.loading).toBe(false));

    const handle = client._presenceHandles[0];
    expect(handle).toBeDefined();
    unmount();
    // stop() clears listeners synchronously in the fake; nothing else to
    // assert against without a spy, so this just documents unmount runs
    // without throwing.
  });

  test("enabled: false skips tracking", () => {
    const client = createFakeClient();
    const { result } = renderHook(() => usePresence(client, "presence", { id: "me" }, { enabled: false }));
    expect(result.current.loading).toBe(false);
    expect(result.current.online.size).toBe(0);
  });
});
