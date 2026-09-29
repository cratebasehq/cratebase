/** Wraps `client.files.upload` (the presigned-direct-upload flow — see
 * `FilesService.upload`'s own doc) as a hook: tracks `progress`/`pending`/
 * `error` for the calling component instead of every caller wiring up its
 * own `useState` around the same `onProgress` callback.
 *
 * ```tsx
 * const { upload, progress, pending } = useUpload();
 * async function onFileChange(file: File) {
 *   const result = await upload(file, { collection: "posts", field: "cover" });
 *   await cb.collection("posts").create({ id: result.recordId, title, cover: result.token });
 * }
 * ```
 */

import { useCallback, useRef, useState } from "react";
import type { CratebaseClient, UploadOptions, UploadResult } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface UseUploadResult {
  /** Starts an upload. Only one is tracked at a time per hook instance —
   * calling this again while one is in flight starts a fresh one and its
   * own `progress`, but does not cancel the previous call (use `abort()`
   * first if that's what's wanted). */
  upload: (file: Blob & { name?: string }, options: UploadOptions) => Promise<UploadResult>;
  /** 0-1 while this hook's own upload is in flight; reset to 0 when a
   * new `upload()` call starts, and left at 1 after it succeeds. */
  progress: number;
  /** `true` from the start of `upload()` until it settles. */
  pending: boolean;
  /** The most recent failure, cleared at the start of the next `upload()`. */
  error: unknown;
  /** Aborts the upload currently in flight, if any — a no-op otherwise.
   * The `upload()` promise then rejects with an `AbortError`. */
  abort: () => void;
}

export function useUpload(client?: CratebaseClient<any>): UseUploadResult {
  const resolved = useResolvedClient(client);
  const [progress, setProgress] = useState(0);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const controllerRef = useRef<AbortController | null>(null);

  const abort = useCallback(() => {
    controllerRef.current?.abort();
  }, []);

  const upload = useCallback(
    async (file: Blob & { name?: string }, options: UploadOptions): Promise<UploadResult> => {
      const controller = new AbortController();
      controllerRef.current = controller;
      // A caller-supplied signal aborts this hook's own controller too,
      // so `FilesService.upload` only ever has to watch one signal.
      const callerSignal = options.signal;
      const onCallerAbort = () => controller.abort();
      if (callerSignal) {
        if (callerSignal.aborted) controller.abort();
        else callerSignal.addEventListener("abort", onCallerAbort);
      }

      setPending(true);
      setError(null);
      setProgress(0);
      try {
        const result = await resolved.files.upload(file, {
          ...options,
          signal: controller.signal,
          onProgress: (fraction: number) => {
            setProgress(fraction);
            options.onProgress?.(fraction);
          },
        });
        setPending(false);
        return result;
      } catch (err) {
        setPending(false);
        setError(err);
        throw err;
      } finally {
        callerSignal?.removeEventListener("abort", onCallerAbort);
        if (controllerRef.current === controller) controllerRef.current = null;
      }
    },
    [resolved],
  );

  return { upload, progress, pending, error, abort };
}
