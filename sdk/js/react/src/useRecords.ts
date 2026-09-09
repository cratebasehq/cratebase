/** Live list of every record in `collectionName` matching `options`:
 * fetches the initial page with `fullList`, then keeps it in sync with a
 * `"*"` realtime subscription, merging create/update/delete events by id —
 * same pattern as `examples/kanban/src/hooks/useCards.ts`. Writes made
 * through `client.collection(collectionName).create()`/`.update()`/`.delete()`
 * (this hook does not wrap writes) land in `records` once their own
 * realtime event arrives, id-merged against whatever is already there. */

import { useCallback, useEffect, useState } from "react";
import type { CratebaseClient, ListOptions, RecordModel, SubscribeOptions } from "@cratebase/client";

export interface UseRecordsOptions<T extends RecordModel> extends ListOptions<T> {
  /** Restricts the realtime subscription to matching events, independent
   * of the initial `fullList` `filter`/`fields`/`expand` above. Defaults
   * to `options.filter`/`options.fields`/`options.expand` when omitted. */
  subscribe?: SubscribeOptions;
  /** Set to `false` to skip fetching/subscribing entirely (e.g. while an
   * id it depends on is not yet known). Defaults to `true`. */
  enabled?: boolean;
}

export interface UseRecordsResult<T extends RecordModel> {
  records: T[];
  loading: boolean;
  error: unknown;
  /** Re-runs the initial `fullList` fetch without touching the realtime
   * subscription. */
  refresh: () => void;
}

export function useRecords<T extends RecordModel = RecordModel>(
  client: CratebaseClient<any>,
  collectionName: string,
  options: UseRecordsOptions<T> = {},
): UseRecordsResult<T> {
  const { subscribe: subscribeOptions, enabled = true, ...listOptions } = options;
  const listOptionsKey = JSON.stringify(listOptions);
  const subscribeOptionsKey = JSON.stringify(
    subscribeOptions ?? { filter: listOptions.filter, fields: listOptions.fields, expand: listOptions.expand },
  );

  const [records, setRecords] = useState<T[]>([]);
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
    setLoading(true);
    setError(null);

    const parsedListOptions = JSON.parse(listOptionsKey) as ListOptions<T>;
    const parsedSubscribeOptions = JSON.parse(subscribeOptionsKey) as SubscribeOptions;
    const service = client.collection(collectionName);

    service
      .fullList(parsedListOptions)
      .then((result) => {
        if (!cancelled) setRecords(result as unknown as T[]);
      })
      .catch((err) => {
        if (!cancelled) setError(err);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    let unsubscribe: (() => void) | undefined;
    service
      .subscribe(
        "*",
        (event) => {
          setRecords((prev) => {
            const record = event.record as unknown as T;
            if (event.action === "delete") return prev.filter((r) => r.id !== record.id);
            const idx = prev.findIndex((r) => r.id === record.id);
            if (idx === -1) return [...prev, record];
            const next = [...prev];
            next[idx] = record;
            return next;
          });
        },
        parsedSubscribeOptions,
      )
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
  }, [client, collectionName, listOptionsKey, subscribeOptionsKey, enabled, refreshNonce]);

  return { records, loading, error, refresh };
}
