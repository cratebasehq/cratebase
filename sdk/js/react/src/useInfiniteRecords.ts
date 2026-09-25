/** "Load more" pagination: accumulates pages of `collectionName` into one
 * growing `records` array, starting at `perPage` (default 20) and
 * fetching the next page on `loadMore()`. Unlike `useRecords`, this hook
 * does not subscribe to realtime by default — refetching from page 1 on
 * every event would silently reset however many pages the caller has
 * already loaded, which is worse than just staying static until the next
 * `refresh()`/`loadMore()`. Pass `realtime: true` to opt in anyway (e.g.
 * an activity feed where "reset to the top on new activity" is exactly
 * the desired behavior). */

import { useCallback, useEffect, useState } from "react";
import type { CratebaseClient, ListOptions, RecordModel } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface UseInfiniteRecordsOptions<T extends RecordModel> extends Omit<ListOptions<T>, "page"> {
  enabled?: boolean;
  /** Refetch every loaded page (from page 1) on any matching realtime
   * event. Defaults to `false` — see this module's doc comment. */
  realtime?: boolean;
}

export interface UseInfiniteRecordsResult<T extends RecordModel> {
  records: T[];
  /** `true` only for the very first page's fetch. */
  loading: boolean;
  /** `true` while a `loadMore()` fetch (page 2+) is in flight. */
  loadingMore: boolean;
  error: unknown;
  /** Whether another `loadMore()` call would return more records. */
  hasMore: boolean;
  loadMore: () => void;
  /** Discards every loaded page and refetches from page 1. */
  refresh: () => void;
}

export function useInfiniteRecords<T extends RecordModel = RecordModel>(
  client: CratebaseClient<any>,
  collectionName: string,
  options?: UseInfiniteRecordsOptions<T>,
): UseInfiniteRecordsResult<T>;
export function useInfiniteRecords<T extends RecordModel = RecordModel>(
  collectionName: string,
  options?: UseInfiniteRecordsOptions<T>,
): UseInfiniteRecordsResult<T>;
export function useInfiniteRecords<T extends RecordModel = RecordModel>(
  arg0: CratebaseClient<any> | string,
  arg1?: string | UseInfiniteRecordsOptions<T>,
  arg2?: UseInfiniteRecordsOptions<T>,
): UseInfiniteRecordsResult<T> {
  const explicitClient = typeof arg0 === "string" ? undefined : arg0;
  const collectionName = (typeof arg0 === "string" ? arg0 : (arg1 as string))!;
  const options: UseInfiniteRecordsOptions<T> =
    (typeof arg0 === "string" ? arg1 : arg2) as UseInfiniteRecordsOptions<T> | undefined ?? {};

  const client = useResolvedClient(explicitClient);
  const { enabled = true, realtime = false, perPage = 20, ...restOptions } = options;
  const optionsKey = JSON.stringify(restOptions);

  const [records, setRecords] = useState<T[]>([]);
  const [page, setPage] = useState(1);
  const [hasMore, setHasMore] = useState(true);
  const [loading, setLoading] = useState(enabled);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [refreshNonce, setRefreshNonce] = useState(0);

  const refresh = useCallback(() => {
    setRecords([]);
    setPage(1);
    setHasMore(true);
    setRefreshNonce((n) => n + 1);
  }, []);

  const loadMore = useCallback(() => {
    setPage((p) => p + 1);
  }, []);

  // Reset to page 1 whenever the query itself changes (a new filter/sort
  // shouldn't append to a list fetched under the old one).
  useEffect(() => {
    setRecords([]);
    setPage(1);
    setHasMore(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, collectionName, optionsKey, perPage, enabled]);

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }

    let cancelled = false;
    const isFirstPage = page === 1;
    if (isFirstPage) setLoading(true);
    else setLoadingMore(true);
    setError(null);

    const parsedOptions = JSON.parse(optionsKey) as Omit<ListOptions<T>, "page" | "perPage">;
    client
      .collection(collectionName)
      .list({ ...parsedOptions, page, perPage })
      .then((result) => {
        if (cancelled) return;
        setRecords((prev) => (isFirstPage ? (result.items as unknown as T[]) : [...prev, ...(result.items as unknown as T[])]));
        setHasMore(page < result.totalPages);
      })
      .catch((err) => {
        if (!cancelled) setError(err);
      })
      .finally(() => {
        if (cancelled) return;
        if (isFirstPage) setLoading(false);
        else setLoadingMore(false);
      });

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, collectionName, optionsKey, perPage, enabled, page, refreshNonce]);

  useEffect(() => {
    if (!enabled || !realtime) return;
    let unsubscribe: (() => void) | undefined;
    let cancelled = false;
    const parsedOptions = JSON.parse(optionsKey) as { filter?: string; fields?: string; expand?: string };

    client
      .collection(collectionName)
      .subscribe("*", () => refresh(), parsedOptions)
      .then((unsub) => {
        if (cancelled) {
          unsub();
          return;
        }
        unsubscribe = unsub;
      })
      .catch((err) => {
        if (!cancelled) setError(err);
      });

    return () => {
      cancelled = true;
      unsubscribe?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, collectionName, optionsKey, enabled, realtime]);

  return { records, loading, loadingMore, error, hasMore, loadMore, refresh };
}
