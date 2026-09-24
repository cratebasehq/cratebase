import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useMutation } from "../useMutation.js";
import { createFakeClient, createFakeCollection } from "./mockClient.js";

describe("useMutation", () => {
  test("create() persists and returns the record", async () => {
    const posts = createFakeCollection<any>([]);
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useMutation(client, "posts"));

    let created: any;
    await act(async () => {
      created = await result.current.create({ title: "A" });
    });

    expect(created.title).toBe("A");
    expect(result.current.pending).toBe(false);
    expect(result.current.error).toBeNull();
  });

  test("pending is true while the request is in flight", async () => {
    const posts = createFakeCollection<any>([]);
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useMutation(client, "posts"));

    let resolveCreate!: (v: any) => void;
    const original = posts.create.bind(posts);
    posts.create = (data: any) => new Promise((resolve) => (resolveCreate = () => resolve(original(data))));

    let createPromise!: Promise<any>;
    act(() => {
      createPromise = result.current.create({ title: "A" });
    });

    await waitFor(() => expect(result.current.pending).toBe(true));

    await act(async () => {
      resolveCreate(undefined);
      await createPromise;
    });

    expect(result.current.pending).toBe(false);
  });

  test("update() and remove() both work", async () => {
    const posts = createFakeCollection<any>([{ id: "p1", title: "A" }]);
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useMutation(client, "posts"));

    await act(async () => {
      const updated = await result.current.update("p1", { title: "B" });
      expect(updated.title).toBe("B");
    });

    await act(async () => {
      await result.current.remove("p1");
    });

    await expect(posts.one("p1")).rejects.toThrow();
  });

  test("a failed create() sets error, calls rollback, and rethrows", async () => {
    const posts = createFakeCollection<any>([]);
    posts.create = async () => {
      throw new Error("boom");
    };
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useMutation(client, "posts"));

    let rolledBack = false;
    let thrown: unknown;
    await act(async () => {
      try {
        await result.current.create(
          { title: "A" },
          { optimistic: { apply: () => {}, rollback: () => (rolledBack = true) } },
        );
      } catch (err) {
        thrown = err;
      }
    });

    expect(rolledBack).toBe(true);
    expect(thrown).toBeInstanceOf(Error);
    expect(result.current.pending).toBe(false);
    expect(result.current.error).toBeInstanceOf(Error);
  });

  test("optimistic.apply() runs synchronously before the request resolves", async () => {
    const posts = createFakeCollection<any>([]);
    const client = createFakeClient({ posts });
    const { result } = renderHook(() => useMutation(client, "posts"));

    let applied = false;
    await act(async () => {
      await result.current.create({ title: "A" }, { optimistic: { apply: () => (applied = true), rollback: () => {} } });
    });

    expect(applied).toBe(true);
  });
});
