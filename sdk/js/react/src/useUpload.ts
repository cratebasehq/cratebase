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
  /** Starts an upload. Overlapping calls are all tracked: `pending` stays
   * `true` until every one has settled and `progress` is the mean of the
   * overlapping uploads' progress. Starting a new one does not cancel the
   * others (use `abort()` for that). */
  upload: (file: Blob & { name?: string }, options: UploadOptions) => Promise<UploadResult>;
  /** 0-1: mean progress of this hook's uploads in the current burst of
   * overlapping calls; reset to 0 when a burst starts, and left at 1 after
   * the last one succeeds. */
  progress: number;
  /** `true` from the start of the first `upload()` until the last one
   * settles. */
  pending: boolean;
  /** The most recent failure, cleared at the start of the next `upload()`. */
  error: unknown;
  /** Aborts every upload currently in flight — a no-op otherwise. Each
   * `upload()` promise then rejects with an `AbortError`. */
  abort: () => void;
}

export function useUpload(client?: CratebaseClient<any>): UseUploadResult {
  const resolved = useResolvedClient(client);
  const [progress, setProgress] = useState(0);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const controllersRef = useRef<Set<AbortController>>(new Set());
  // Per-upload fraction for the current burst of overlapping uploads.
  const fractionsRef = useRef<Map<AbortController, number>>(new Map());

  const publishProgress = useCallback(() => {
    const values = [...fractionsRef.current.values()];
    setProgress(values.length === 0 ? 0 : values.reduce((a, b) => a + b, 0) / values.length);
  }, []);

  const abort = useCallback(() => {
    for (const controller of [...controllersRef.current]) controller.abort();
  }, []);

  const upload = useCallback(
    async (file: Blob & { name?: string }, options: UploadOptions): Promise<UploadResult> => {
      const controller = new AbortController();
      if (controllersRef.current.size === 0) fractionsRef.current.clear();
      controllersRef.current.add(controller);
      fractionsRef.current.set(controller, 0);
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
      publishProgress();
      try {
        const result = await resolved.files.upload(file, {
          ...options,
          signal: controller.signal,
          onProgress: (fraction: number) => {
            if (fractionsRef.current.has(controller)) {
              fractionsRef.current.set(controller, fraction);
              publishProgress();
            }
            options.onProgress?.(fraction);
          },
        });
        fractionsRef.current.set(controller, 1);
        return result;
      } catch (err) {
        fractionsRef.current.delete(controller);
        setError(err);
        throw err;
      } finally {
        callerSignal?.removeEventListener("abort", onCallerAbort);
        controllersRef.current.delete(controller);
        publishProgress();
        if (controllersRef.current.size === 0) setPending(false);
      }
    },
    [resolved, publishProgress],
  );

  return { upload, progress, pending, error, abort };
}
