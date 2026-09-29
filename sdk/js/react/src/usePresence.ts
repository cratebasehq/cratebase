/** Tracks this client's own presence on a realtime channel
 * (`crates/server/src/realtime.rs`'s `POST .../presence` heartbeat) for
 * as long as the component stays mounted and `state` is non-`null`, and
 * exposes the live member list. Server-authoritative: membership is
 * removed automatically when this tab's SSE connection drops, no
 * `staleMs` window to tune — see `@cratebase/client`'s
 * `Channel["presence"]` for the full contract.
 *
 * This is a different hook from the pre-realtime-channels
 * `useRecordPresence` (this name, `usePresence`, used to mean that one —
 * see its own doc comment and the CHANGELOG for the rename). */

import { useEffect, useRef, useState } from "react";
import type { CratebaseClient, PresenceEventKind, PresenceMember } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

/** How often to re-send the heartbeat while `state` is non-`null` —
 * comfortably under a third of the server's presence TTL (45s as of
 * `crates/server/src/realtime.rs`'s `PRESENCE_TTL`), so one missed beat
 * from a slow network never flaps this client's own membership. */
const DEFAULT_HEARTBEAT_MS = 12_000;

export interface UsePresenceOptions<T> {
  /** Set to `false`, or leave `state` as `null`, to observe the channel's
   * presence without tracking any of your own. Defaults to `true`. */
  enabled?: boolean;
  heartbeatMs?: number;
  onChange?: (kind: PresenceEventKind, member: PresenceMember<T>) => void;
}

export interface UsePresenceResult<T> {
  /** The channel's current members, kept live via `presence.join`/
   * `.update`/`.leave`, seeded with a fresh `list()` on connect. */
  members: PresenceMember<T>[];
  loading: boolean;
  error: unknown;
}

export function usePresence<T = unknown>(
  client: CratebaseClient<any>,
  channel: string,
  state: T | null,
  options?: UsePresenceOptions<T>,
): UsePresenceResult<T>;
export function usePresence<T = unknown>(
  channel: string,
  state: T | null,
  options?: UsePresenceOptions<T>,
): UsePresenceResult<T>;
export function usePresence<T = unknown>(
  arg0: CratebaseClient<any> | string,
  arg1: string | T | null,
  arg2?: T | null | UsePresenceOptions<T>,
  arg3?: UsePresenceOptions<T>,
): UsePresenceResult<T> {
  const isClient = typeof arg0 !== "string";
  const explicitClient = isClient ? (arg0 as CratebaseClient<any>) : undefined;
  const channelName = (isClient ? (arg1 as string) : arg0)!;
  const state = (isClient ? arg2 : arg1) as T | null;
  const options = ((isClient ? arg3 : (arg2 as UsePresenceOptions<T> | undefined)) ?? {}) as UsePresenceOptions<T>;

  const client = useResolvedClient(explicitClient);
  const { enabled = true, heartbeatMs = DEFAULT_HEARTBEAT_MS, onChange } = options;
  const tracking = enabled && state !== null && state !== undefined;
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const stateKey = JSON.stringify(state);

  const [members, setMembers] = useState<PresenceMember<T>[]>([]);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<unknown>(null);

  // Subscribe to this channel's presence events and seed the list.
  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    const channel = client.channel<T>(channelName);

    let unsubscribe: (() => void) | undefined;
    channel.presence
      .onChange((kind, member) => {
        onChangeRef.current?.(kind, member);
        setMembers((current) => {
          const others = current.filter((m) => m.clientId !== member.clientId);
          return kind === "leave" ? others : [...others, member];
        });
      })
      .then((unsub) => {
        if (cancelled) {
          unsub();
          return undefined;
        }
        unsubscribe = unsub;
        return channel.presence.list();
      })
      .then((list) => {
        if (cancelled || !list) return;
        setMembers(list);
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
      unsubscribe?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, channelName, enabled]);

  // Heartbeat this client's own state while `tracking`.
  useEffect(() => {
    if (!tracking) return;
    let cancelled = false;
    const channel = client.channel<T>(channelName);
    const beat = () => {
      channel.presence.track(state as T).catch((err) => {
        if (!cancelled) setError(err);
      });
    };
    beat();
    const interval = setInterval(beat, heartbeatMs);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, channelName, tracking, stateKey, heartbeatMs]);

  return { members, loading, error };
}
