/** Wraps `client.presence.track` (a client-side heartbeat + realtime +
 * staleness pattern over an ordinary collection — see
 * `CratebaseClient["presence"]`'s doc comment) as a hook: tracks `data`
 * as this client's own row for as long as the component stays mounted,
 * and exposes the live `online` set of every fresh peer.
 *
 * This is the pre-realtime-channels presence pattern, renamed from
 * `usePresence` (which now means the server-authoritative,
 * `_channels`-backed presence in `useChannel.ts`/`usePresence.ts` — see
 * the CHANGELOG). It still works exactly as before and has no planned
 * removal: it needs no `_channels` row (any collection with a rule that
 * lets peers list each other works), which suits an app that already has
 * a natural "peers" collection (a `_team_members`-shaped table, a game's
 * `players`) more than an ad hoc channel would. Prefer the new
 * `usePresence`/`useChannel` for anything else — no backing collection to
 * create, no client-side staleness window to tune, and membership
 * disappears automatically when a tab closes rather than after `staleMs`. */

import { useEffect, useRef, useState } from "react";
import type { CratebaseClient, Presence, PresenceOptions } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface UseRecordPresenceOptions extends PresenceOptions {
  /** Set to `false` to skip tracking/observing entirely (e.g. before the
   * caller has an id to publish under). Defaults to `true`. */
  enabled?: boolean;
}

export interface UseRecordPresenceResult {
  /** ids of the presence rows currently considered online, including
   * this client's own once its first heartbeat lands. */
  online: Set<string>;
  loading: boolean;
  error: unknown;
}

export function useRecordPresence(
  client: CratebaseClient<any>,
  collectionName: string,
  data: Record<string, unknown> & { id?: string },
  options?: UseRecordPresenceOptions,
): UseRecordPresenceResult;
export function useRecordPresence(
  collectionName: string,
  data: Record<string, unknown> & { id?: string },
  options?: UseRecordPresenceOptions,
): UseRecordPresenceResult;
export function useRecordPresence(
  arg0: CratebaseClient<any> | string,
  arg1: string | (Record<string, unknown> & { id?: string }),
  arg2?: (Record<string, unknown> & { id?: string }) | UseRecordPresenceOptions,
  arg3?: UseRecordPresenceOptions,
): UseRecordPresenceResult {
  const explicitClient = typeof arg0 === "string" ? undefined : arg0;
  const collectionName = (typeof arg0 === "string" ? arg0 : (arg1 as string))!;
  const data = (typeof arg0 === "string" ? arg1 : arg2) as Record<string, unknown> & { id?: string };
  const options: UseRecordPresenceOptions =
    ((typeof arg0 === "string" ? arg2 : arg3) as UseRecordPresenceOptions | undefined) ?? {};

  const client = useResolvedClient(explicitClient);
  const { enabled = true, ...presenceOptions } = options;
  const dataKey = JSON.stringify(data);
  const presenceOptionsKey = JSON.stringify(presenceOptions);

  const [online, setOnline] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<unknown>(null);
  const presenceRef = useRef<Presence | null>(null);

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }

    let cancelled = false;
    let unsubscribeOnline: (() => void) | undefined;
    setLoading(true);
    setError(null);

    client.presence
      .track(collectionName, JSON.parse(dataKey), JSON.parse(presenceOptionsKey))
      .then((presence) => {
        if (cancelled) {
          void presence.stop();
          return;
        }
        presenceRef.current = presence;
        unsubscribeOnline = presence.subscribe(setOnline);
        setLoading(false);
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err);
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
      unsubscribeOnline?.();
      void presenceRef.current?.stop();
      presenceRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, collectionName, dataKey, presenceOptionsKey, enabled]);

  return { online, loading, error };
}
