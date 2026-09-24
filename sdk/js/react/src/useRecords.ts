/** Live list of records in `collectionName` matching `options`: fetches
 * with `fullList` (no `page`/`perPage` given) or a single `list()` page
 * (either given), then — when `realtime` is enabled (the default) —
 * refetches on every matching realtime event rather than patching
 * `records` locally.
 *
 * Refetch-on-event, not local create/update/delete patching: a patched
 * list has to reimplement the server's own `filter`/`sort`/pagination
 * semantics client-side to stay correct (an update that makes a row stop
 * matching `filter` must remove it; an insert must land at the right
 * sorted position; a paginated page must not silently grow past
 * `perPage`), which is exactly the kind of thing that quietly drifts from
 * the server's actual behavior. Refetching after a short debounce keeps
 * this hook's list always exactly what the server would return for the
 * same query, at the cost of one extra request per (coalesced) burst of
 * writes — correct first, since the realtime event here is only ever
 * used as an invalidation signal, not as list data. */

import { useCallback, useEffect, useState } from "react";
import type { CratebaseClient, ListOptions, RecordModel, SubscribeOptions } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

/** How long to wait after a realtime event before refetching, coalescing
 * a burst of events (e.g. a batch write) into a single request. */
const REFETCH_DEBOUNCE_MS = 50;

export interface UseRecordsOptions<T extends RecordModel> extends ListOptions<T> {
  /** Restricts the realtime subscription to matching events, independent
   * of the initial fetch's `filter`/`fields`/`expand` above. Defaults
   * to `options.filter`/`options.fields`/`options.expand` when omitted. */
  subscribe?: SubscribeOptions;
  /** Set to `false` to skip fetching/subscribing entirely (e.g. while an
   * id it depends on is not yet known). Defaults to `true`. */
  enabled?: boolean;
  /** Keep `records` live by refetching on every matching realtime event.
   * Defaults to `true`. Set to `false` for a one-shot fetch with no
   * subscription (`refresh()` remains available to refetch manually). */
  realtime?: boolean;
}

export interface UseRecordsResult<T extends RecordModel> {
  records: T[];
  loading: boolean;
  error: unknown;
  /** Re-runs the fetch without touching the realtime subscription. */
  refresh: () => void;
  /** `1` and `records.length` respectively when `page`/`perPage` were not
   * passed (i.e. this hook fetched with `fullList`). */
  page: number;
  perPage: number;
  totalItems: number;
  totalPages: number;
}

const UNPAGINATED_META = { page: 1, totalPages: 1 };

export function useRecords<T extends RecordModel = RecordModel>(
  client: CratebaseClient<any>,
  collectionName: string,
  options?: UseRecordsOptions<T>,
): UseRecordsResult<T>;
export function useRecords<T extends RecordModel = RecordModel>(
  collectionName: string,
  options?: UseRecordsOptions<T>,
): UseRecordsResult<T>;
export function useRecords<T extends RecordModel = RecordModel>(
  arg0: CratebaseClient<any> | string,
  arg1?: string | UseRecordsOptions<T>,
  arg2?: UseRecordsOptions<T>,
): UseRecordsResult<T> {
  const explicitClient = typeof arg0 === "string" ? undefined : arg0;
  const collectionName = (typeof arg0 === "string" ? arg0 : (arg1 as string))!;
  const options: UseRecordsOptions<T> = (typeof arg0 === "string" ? arg1 : arg2) as UseRecordsOptions<T> | undefined ?? {};

  const client = useResolvedClient(explicitClient);
  const { subscribe: subscribeOptions, enabled = true, realtime = true, ...listOptions } = options;
  const paginated = listOptions.page !== undefined || listOptions.perPage !== undefined;
  const listOptionsKey = JSON.stringify(listOptions);
  const subscribeOptionsKey = JSON.stringify(
    subscribeOptions ?? { filter: listOptions.filter, fields: listOptions.fields, expand: listOptions.expand },
  );

  const [records, setRecords] = useState<T[]>([]);
  const [meta, setMeta] = useState({ page: 1, perPage: 0, totalItems: 0, totalPages: 1 });
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<unknown>(null);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const refresh = useCallback(() => setRefreshNonce((n) => n + 1), []);

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }

    let cancelled = false;
    const parsedListOptions = JSON.parse(listOptionsKey) as ListOptions<T>;
    const parsedSubscribeOptions = JSON.parse(subscribeOptionsKey) as SubscribeOptions;
    const service = client.collection(collectionName);

    const fetchOnce = () => {
      setLoading(true);
      setError(null);
      const promise = paginated
        ? service.list(parsedListOptions).then((result) => {
            if (cancelled) return;
            setRecords(result.items as unknown as T[]);
            setMeta({ page: result.page, perPage: result.perPage, totalItems: result.totalItems, totalPages: result.totalPages });
          })
        : service.fullList(parsedListOptions).then((result) => {
            if (cancelled) return;
            setRecords(result as unknown as T[]);
            setMeta((m) => ({ ...m, ...UNPAGINATED_META, perPage: result.length, totalItems: result.length }));
          });
      promise
        .catch((err) => {
          if (!cancelled) setError(err);
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    };

    fetchOnce();

    let unsubscribe: (() => void) | undefined;
    if (realtime) {
      let debounceTimer: ReturnType<typeof setTimeout> | undefined;
      const scheduleRefetch = () => {
        if (debounceTimer !== undefined) return;
        debounceTimer = setTimeout(() => {
          debounceTimer = undefined;
          if (!cancelled) fetchOnce();
        }, REFETCH_DEBOUNCE_MS);
      };

      service
        .subscribe("*", scheduleRefetch, parsedSubscribeOptions)
        .then((unsub) => {
          if (cancelled) {
            unsub();
            return;
          }
          unsubscribe = () => {
            if (debounceTimer !== undefined) clearTimeout(debounceTimer);
            unsub();
          };
        })
        .catch((err) => {
          if (!cancelled) setError(err);
        });
    }

    return () => {
      cancelled = true;
      unsubscribe?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, collectionName, listOptionsKey, subscribeOptionsKey, enabled, realtime, paginated, refreshNonce]);

  return { records, loading, error, refresh, ...meta };
}
