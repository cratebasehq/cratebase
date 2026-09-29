import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useUpload } from "../useUpload.js";
import { createFakeClient, createFakeFiles } from "./mockClient.js";

describe("useUpload", () => {
  test("upload() resolves with the token/recordId/filename and reports progress", async () => {
    const client = createFakeClient({});
    const { result } = renderHook(() => useUpload(client));

    let uploadResult: any;
    await act(async () => {
      uploadResult = await result.current.upload(new Blob(["hi"], { type: "text/plain" }) as any, {
        collection: "posts",
        field: "cover",
      });
    });

    expect(uploadResult.token).toBe("tok_upload_1");
    expect(uploadResult.recordId).toBe("rec_upload_1");
    expect(result.current.pending).toBe(false);
    expect(result.current.progress).toBe(1);
    expect(result.current.error).toBeNull();
  });

  test("pending is true while the upload is in flight", async () => {
    const files = createFakeFiles();
    let resolveUpload!: () => void;
    const originalUpload = files.upload.bind(files);
    files.upload = (file, options: any) =>
      new Promise((resolve) => {
        resolveUpload = () => resolve(originalUpload(file, options));
      });
    const client = createFakeClient({}, { files });
    const { result } = renderHook(() => useUpload(client));

    let uploadPromise!: Promise<any>;
    act(() => {
      uploadPromise = result.current.upload(new Blob(["hi"]) as any, { collection: "posts", field: "cover" });
    });

    await waitFor(() => expect(result.current.pending).toBe(true));

    await act(async () => {
      resolveUpload();
      await uploadPromise;
    });

    expect(result.current.pending).toBe(false);
  });

  test("the hook's own onProgress composes with a caller-supplied one", async () => {
    const client = createFakeClient({});
    const { result } = renderHook(() => useUpload(client));

    const seen: number[] = [];
    await act(async () => {
      await result.current.upload(new Blob(["hi"]) as any, {
        collection: "posts",
        field: "cover",
        onProgress: (f) => seen.push(f),
      });
    });

    expect(seen).toEqual([0.5, 1]);
    expect(result.current.progress).toBe(1);
  });

  test("a failed upload sets error, clears pending, and rethrows", async () => {
    const files = createFakeFiles();
    files.upload = async () => {
      throw new Error("boom");
    };
    const client = createFakeClient({}, { files });
    const { result } = renderHook(() => useUpload(client));

    let thrown: unknown;
    await act(async () => {
      try {
        await result.current.upload(new Blob(["hi"]) as any, { collection: "posts", field: "cover" });
      } catch (err) {
        thrown = err;
      }
    });

    expect(thrown).toBeInstanceOf(Error);
    expect(result.current.pending).toBe(false);
    expect(result.current.error).toBeInstanceOf(Error);
  });

  test("abort() aborts the in-flight upload", async () => {
    const files = createFakeFiles();
    files.upload = (_file, options: any) =>
      new Promise((_resolve, reject) => {
        options.signal?.addEventListener("abort", () => reject(new DOMException("Upload aborted.", "AbortError")));
      });
    const client = createFakeClient({}, { files });
    const { result } = renderHook(() => useUpload(client));

    let uploadPromise!: Promise<any>;
    act(() => {
      uploadPromise = result.current.upload(new Blob(["hi"]) as any, { collection: "posts", field: "cover" });
    });

    await waitFor(() => expect(result.current.pending).toBe(true));

    let thrown: unknown;
    await act(async () => {
      result.current.abort();
      try {
        await uploadPromise;
      } catch (err) {
        thrown = err;
      }
    });

    expect(thrown).toBeInstanceOf(DOMException);
    expect(result.current.pending).toBe(false);
  });
});
