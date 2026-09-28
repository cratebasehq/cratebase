/** Live list of the signed-in caller's own in-app notifications
 * (`_notifications`, scoped by `owner_rule` — see `crates/core/src/
 * collection.rs`'s `default_system_collections` comment on
 * `notifications`), plus the unread count and the two bulk actions a
 * notification bell needs. Same "refetch on realtime event, debounced"
 * shape as `useRecords` — see that hook's doc comment for why patching
 * the list locally isn't the right default. */

import { useCallback, useEffect, useState } from "react";
import type { CratebaseClient, NotificationRecord } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

/** How long to wait after a realtime event before refetching, coalescing
 * a burst of events into one request — same constant `useRecords` uses. */
const REFETCH_DEBOUNCE_MS = 50;

export interface UseNotificationsOptions {
  /** Set to `false` to skip fetching/subscribing entirely (e.g. before
   * the caller is signed in). Defaults to `true`. */
  enabled?: boolean;
  /** Keep `items`/`unreadCount` live by refetching on every realtime
   * event for this recipient. Defaults to `true`. */
  realtime?: boolean;
  /** Caps how many notifications `items` holds (most recent first).
   * Defaults to 50 — a bell dropdown, not a full inbox; page through
   * `client.notifications.list()` directly for that. */
  limit?: number;
}

export interface UseNotificationsResult {
  items: NotificationRecord[];
  unreadCount: number;
  loading: boolean;
  error: unknown;
  /** Re-fetches `items`/`unreadCount` without touching the subscription. */
  refresh: () => void;
  markRead: (id: string) => Promise<void>;
  markAllRead: () => Promise<void>;
}

const DEFAULT_LIMIT = 50;

export function useNotifications(client: CratebaseClient<any>, options?: UseNotificationsOptions): UseNotificationsResult;
export function useNotifications(options?: UseNotificationsOptions): UseNotificationsResult;
export function useNotifications(
  arg0?: CratebaseClient<any> | UseNotificationsOptions,
  arg1?: UseNotificationsOptions,
): UseNotificationsResult {
  const isClient = !!arg0 && typeof (arg0 as CratebaseClient<any>).collection === "function";
  const explicitClient = isClient ? (arg0 as CratebaseClient<any>) : undefined;
  const options: UseNotificationsOptions = (isClient ? arg1 : (arg0 as UseNotificationsOptions | undefined)) ?? {};

  const client = useResolvedClient(explicitClient);
  const { enabled = true, realtime = true, limit = DEFAULT_LIMIT } = options;

  const [items, setItems] = useState<NotificationRecord[]>([]);
  const [unreadCount, setUnreadCount] = useState(0);
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

    const fetchOnce = () => {
      setLoading(true);
      setError(null);
      Promise.all([
        client.notifications.list({ perPage: limit, sort: "-created" }),
        client.notifications.unreadCount(),
      ])
        .then(([list, count]) => {
          if (cancelled) return;
          setItems(list.items);
          setUnreadCount(count);
        })
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

      client.notifications
        .subscribe(scheduleRefetch)
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
  }, [client, enabled, realtime, limit, refreshNonce]);

  const markRead = useCallback(
    async (id: string) => {
      await client.notifications.markRead(id);
      refresh();
    },
    [client, refresh],
  );

  const markAllRead = useCallback(async () => {
    await client.notifications.markAllRead();
    refresh();
  }, [client, refresh]);

  return { items, unreadCount, loading, error, refresh, markRead, markAllRead };
}
