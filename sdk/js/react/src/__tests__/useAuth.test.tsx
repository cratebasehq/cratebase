import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useAuth } from "../useAuth.js";
import { CratebaseProvider } from "../context.js";
import { createFakeClient } from "./mockClient.js";

describe("useAuth", () => {
  test("starts signed out", () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useAuth(client));
    expect(result.current.isValid).toBe(false);
    expect(result.current.user).toBeNull();
    expect(result.current.isLoading).toBe(false);
  });

  test("re-renders on sign-in via client.auth.signIn.password", async () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useAuth(client));

    await act(async () => {
      await result.current.signIn.password({ identity: "a@b.com", password: "x" });
    });

    await waitFor(() => expect(result.current.isValid).toBe(true));
    expect(result.current.user?.email).toBe("a@b.com");
  });

  test("re-renders on an out-of-band auth change (not through this hook's own signIn)", async () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useAuth(client));

    act(() => {
      client.auth._signInAs({ id: "u1", collectionId: "users", collectionName: "users" });
    });

    await waitFor(() => expect(result.current.isValid).toBe(true));
  });

  test("signOut clears state", async () => {
    const client = createFakeClient();
    client.auth._signInAs({ id: "u1", collectionId: "users", collectionName: "users" });
    const { result } = renderHook(() => useAuth(client));
    expect(result.current.isValid).toBe(true);

    await act(async () => {
      await result.current.signOut();
    });

    await waitFor(() => expect(result.current.isValid).toBe(false));
    expect(result.current.user).toBeNull();
  });

  test("isSuperuser reflects the _superusers collection", () => {
    const client = createFakeClient();
    client.auth._signInAs({ id: "u1", collectionId: "_superusers", collectionName: "_superusers" });
    const { result } = renderHook(() => useAuth(client));
    expect(result.current.isSuperuser).toBe(true);
  });

  test("works via CratebaseProvider context with no explicit client", () => {
    const client = createFakeClient();
    client.auth._signInAs({ id: "u1", collectionId: "users", collectionName: "users" });

    const { result } = renderHook(() => useAuth(), {
      wrapper: ({ children }) => <CratebaseProvider client={client}>{children}</CratebaseProvider>,
    });

    expect(result.current.isValid).toBe(true);
  });
});
