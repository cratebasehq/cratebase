import { describe, expect, test, beforeEach } from "bun:test";
import { renderHook, waitFor } from "@testing-library/react";
import { useMagicLinkCallback } from "../useMagicLinkCallback.js";
import { createFakeClient } from "./mockClient.js";

function setUrl(path: string) {
  window.history.pushState({}, "", path);
}

describe("useMagicLinkCallback", () => {
  beforeEach(() => {
    setUrl("/callback");
  });

  test("status is 'none' when the URL has no token", () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useMagicLinkCallback(client));
    expect(result.current.status).toBe("none");
  });

  test("signs in from a ?token= param and strips it from the URL", async () => {
    setUrl("/callback?token=good-token&x=1");
    const client = createFakeClient();
    const { result } = renderHook(() => useMagicLinkCallback(client));

    await waitFor(() => expect(result.current.status).toBe("success"));
    expect(client.auth.isValid).toBe(true);
    expect(client.auth.record?.email).toBe("ml@example.com");
    expect(window.location.search).toContain("x=1");
    expect(window.location.search).not.toContain("token=");
  });

  test("reports 'error' for an invalid token without throwing", async () => {
    setUrl("/callback?token=bad-token");
    const client = createFakeClient();
    const onError = () => {};
    const { result } = renderHook(() => useMagicLinkCallback(client, { onError }));

    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error).toBeDefined();
    expect(client.auth.isValid).toBe(false);
  });

  test("respects a custom param name", async () => {
    setUrl("/callback?mlToken=good-token");
    const client = createFakeClient();
    const { result } = renderHook(() => useMagicLinkCallback(client, { param: "mlToken" }));

    await waitFor(() => expect(result.current.status).toBe("success"));
  });
});
