import { describe, expect, test } from "bun:test";
import { renderHook } from "@testing-library/react";
import { CratebaseProvider, useCratebase } from "../context.js";
import { createFakeClient } from "./mockClient.js";

describe("CratebaseProvider / useCratebase", () => {
  test("useCratebase() returns the provided client", () => {
    const client = createFakeClient();
    const { result } = renderHook(() => useCratebase(), {
      wrapper: ({ children }) => <CratebaseProvider client={client}>{children}</CratebaseProvider>,
    });
    expect(result.current).toBe(client);
  });

  test("useCratebase() throws outside a provider", () => {
    const originalError = console.error;
    console.error = () => {};
    try {
      expect(() => renderHook(() => useCratebase())).toThrow(/CratebaseProvider/);
    } finally {
      console.error = originalError;
    }
  });
});
