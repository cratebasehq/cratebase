/** Live single record: `id`'s single-record variant of `useRecords`,
 * fetching once with `one()` and then subscribing to that record's own
 * topic (`${collectionName}/id`, via `CollectionService.subscribe(id, …)`)
 * rather than `"*"` — the server only pushes events for this one record. */

import { useEffect, useState } from "react";
import type { CratebaseClient, RecordModel, ViewOptions } from "@cratebase/client";

export interface UseRecordOptions extends ViewOptions {
  /** Set to `false` to skip fetching/subscribing entirely (e.g. while
   * `id` is not yet known). Defaults to `true`. */
  enabled?: boolean;
}

export interface UseRecordResult<T extends RecordModel> {
  record: T | null;
  loading: boolean;
  error: unknown;
  /** `true` once the record's own `delete` realtime event has been
   * observed — `record` still holds its last known value so callers can
   * render a "this was deleted" state instead of a blank one. */
  deleted: boolean;
}

export function useRecord<T extends RecordModel = RecordModel>(
  client: CratebaseClient<any>,
  collectionName: string,
  id: string | null | undefined,
  options: UseRecordOptions = {},
): UseRecordResult<T> {
  const { enabled = true, ...viewOptions } = options;
  const viewOptionsKey = JSON.stringify(viewOptions);

  const [record, setRecord] = useState<T | null>(null);
  const [loading, setLoading] = useState(Boolean(enabled && id));
  const [error, setError] = useState<unknown>(null);
  const [deleted, setDeleted] = useState(false);

  useEffect(() => {
    if (!enabled || !id) {
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);
    setDeleted(false);

    const parsedViewOptions = JSON.parse(viewOptionsKey) as ViewOptions;
    const service = client.collection(collectionName);

    service
      .one(id, parsedViewOptions)
      .then((result) => {
        if (!cancelled) setRecord(result as unknown as T);
      })
      .catch((err) => {
        if (!cancelled) setError(err);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    let unsubscribe: (() => void) | undefined;
    service
      .subscribe(id, (event) => {
        if (event.action === "delete") {
          setDeleted(true);
          return;
        }
        setRecord(event.record as unknown as T);
      })
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
  }, [client, collectionName, id, viewOptionsKey, enabled]);

  return { record, loading, error, deleted };
}
