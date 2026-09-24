/** Runtime behavior of `createCratebaseHooks` — the inference itself
 * (compile-time) is covered separately by `../inference.typecheck.ts`
 * via `tsc -p tsconfig.typecheck.json`; this file only checks that the
 * returned hooks actually work when called, wired through context. */

import { describe, expect, test } from "bun:test";
import { renderHook, waitFor } from "@testing-library/react";
import { createCratebaseHooks } from "../createHooks.js";
import { CratebaseProvider } from "../context.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

type Schema = {
  posts: { title: string };
};

describe("createCratebaseHooks", () => {
  test("useRecords resolves the client from context and fetches", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });
    const { useRecords } = createCratebaseHooks<Schema>();

    const { result } = renderHook(() => useRecords("posts"), {
      wrapper: ({ children }) => <CratebaseProvider client={client}>{children}</CratebaseProvider>,
    });

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.records).toHaveLength(1);
  });

  test("useCratebase returns the same client", () => {
    const client = createFakeClient();
    const { useCratebase } = createCratebaseHooks<Schema>();
    const { result } = renderHook(() => useCratebase(), {
      wrapper: ({ children }) => <CratebaseProvider client={client}>{children}</CratebaseProvider>,
    });
    expect(result.current).toBe(client);
  });

  test("useMutation creates through the resolved client", async () => {
    const posts = createFakeCollection<any>([]);
    const client = createFakeClient({ posts });
    const { useMutation } = createCratebaseHooks<Schema>();

    const { result } = renderHook(() => useMutation("posts"), {
      wrapper: ({ children }) => <CratebaseProvider client={client}>{children}</CratebaseProvider>,
    });

    const created = await result.current.create({ title: "A" });
    expect(created.title).toBe("A");
  });
});
