/** `create`/`update`/`remove` for one collection, with `pending`/`error`
 * state and an optional apply/rollback pair for optimistic UI:
 *
 * ```ts
 * const { create, pending } = useMutation("posts");
 * await create(
 *   { title },
 *   { optimistic: { apply: () => setLocal((p) => [...p, draft]), rollback: () => setLocal((p) => p.filter((x) => x !== draft)) } },
 * );
 * ```
 *
 * `optimistic.apply()` runs synchronously before the request is sent;
 * `optimistic.rollback()` runs if the request throws. Neither hook
 * touches any `useRecords`/`useRecord` state on its own — this stays a
 * standalone primitive callers wire up to whatever local state they're
 * showing optimistically (a component's own `useState`, a list from
 * `useRecords`'s own `records` mirrored into local state, etc.). Paired
 * with `useRecords({ realtime: true })`, the realtime event this
 * mutation's own write produces settles the list with the server's real
 * value shortly after, the same way any other write does. */

import { useCallback, useRef, useState } from "react";
import type { CratebaseClient, RecordModel, WriteOptions } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface OptimisticOptions {
  /** Runs synchronously before the request is sent. */
  apply: () => void;
  /** Runs if the request throws — typically the inverse of `apply`. */
  rollback: () => void;
}

export interface MutationWriteOptions extends WriteOptions {
  optimistic?: OptimisticOptions;
}

export interface UseMutationResult<T extends RecordModel> {
  create: (data: Partial<T> | FormData, options?: MutationWriteOptions) => Promise<T>;
  update: (id: string, data: Partial<T> | FormData, options?: MutationWriteOptions) => Promise<T>;
  remove: (id: string, options?: { optimistic?: OptimisticOptions }) => Promise<void>;
  /** `true` while any `create`/`update`/`remove` call from this hook
   * instance is in flight. */
  pending: boolean;
  /** The most recent failure, cleared at the start of the next call. */
  error: unknown;
}

export function useMutation<T extends RecordModel = RecordModel>(
  client: CratebaseClient<any>,
  collectionName: string,
): UseMutationResult<T>;
export function useMutation<T extends RecordModel = RecordModel>(collectionName: string): UseMutationResult<T>;
export function useMutation<T extends RecordModel = RecordModel>(
  arg0: CratebaseClient<any> | string,
  arg1?: string,
): UseMutationResult<T> {
  const explicitClient = typeof arg0 === "string" ? undefined : arg0;
  const collectionName = (typeof arg0 === "string" ? arg0 : arg1)!;

  const client = useResolvedClient(explicitClient);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  // Tracks concurrent in-flight calls so `pending` only drops back to
  // `false` once every one of them has settled.
  const inFlight = useRef(0);

  const begin = useCallback(() => {
    inFlight.current += 1;
    setPending(true);
    setError(null);
  }, []);
  const end = useCallback((err?: unknown) => {
    inFlight.current -= 1;
    if (inFlight.current <= 0) {
      inFlight.current = 0;
      setPending(false);
    }
    if (err !== undefined) setError(err);
  }, []);

  const create = useCallback(
    async (data: Partial<T> | FormData, options: MutationWriteOptions = {}): Promise<T> => {
      const { optimistic, ...writeOptions } = options;
      optimistic?.apply();
      begin();
      try {
        const result = await client.collection(collectionName).create(data, writeOptions);
        end();
        return result as unknown as T;
      } catch (err) {
        optimistic?.rollback();
        end(err);
        throw err;
      }
    },
    [client, collectionName, begin, end],
  );

  const update = useCallback(
    async (id: string, data: Partial<T> | FormData, options: MutationWriteOptions = {}): Promise<T> => {
      const { optimistic, ...writeOptions } = options;
      optimistic?.apply();
      begin();
      try {
        const result = await client.collection(collectionName).update(id, data, writeOptions);
        end();
        return result as unknown as T;
      } catch (err) {
        optimistic?.rollback();
        end(err);
        throw err;
      }
    },
    [client, collectionName, begin, end],
  );

  const remove = useCallback(
    async (id: string, options: { optimistic?: OptimisticOptions } = {}): Promise<void> => {
      const { optimistic } = options;
      optimistic?.apply();
      begin();
      try {
        await client.collection(collectionName).delete(id);
        end();
      } catch (err) {
        optimistic?.rollback();
        end(err);
        throw err;
      }
    },
    [client, collectionName, begin, end],
  );

  return { create, update, remove, pending, error };
}
