/** Wraps `client.presence.track` (`crates` has no server-side presence
 * concept — this is the heartbeat + realtime + client-side staleness
 * pattern documented on `CratebaseClient["presence"]`) as a hook: tracks
 * `data` as this client's own row for as long as the component stays
 * mounted, and exposes the live `online` set of every fresh peer. */

import { useEffect, useRef, useState } from "react";
import type { CratebaseClient, Presence, PresenceOptions } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface UsePresenceOptions extends PresenceOptions {
  /** Set to `false` to skip tracking/observing entirely (e.g. before the
   * caller has an id to publish under). Defaults to `true`. */
  enabled?: boolean;
}

export interface UsePresenceResult {
  /** ids of the presence rows currently considered online, including
   * this client's own once its first heartbeat lands. */
  online: Set<string>;
  loading: boolean;
  error: unknown;
}

export function usePresence(
  client: CratebaseClient<any>,
  collectionName: string,
  data: Record<string, unknown> & { id?: string },
  options?: UsePresenceOptions,
): UsePresenceResult;
export function usePresence(
  collectionName: string,
  data: Record<string, unknown> & { id?: string },
  options?: UsePresenceOptions,
): UsePresenceResult;
export function usePresence(
  arg0: CratebaseClient<any> | string,
  arg1: string | (Record<string, unknown> & { id?: string }),
  arg2?: (Record<string, unknown> & { id?: string }) | UsePresenceOptions,
  arg3?: UsePresenceOptions,
): UsePresenceResult {
  const explicitClient = typeof arg0 === "string" ? undefined : arg0;
  const collectionName = (typeof arg0 === "string" ? arg0 : (arg1 as string))!;
  const data = (typeof arg0 === "string" ? arg1 : arg2) as Record<string, unknown> & { id?: string };
  const options: UsePresenceOptions = ((typeof arg0 === "string" ? arg2 : arg3) as UsePresenceOptions | undefined) ?? {};

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
