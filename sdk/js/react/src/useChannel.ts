/** Subscribes to a realtime channel (`crates/server/src/realtime.rs`'s
 * `channel:<name>` topic — not tied to any record/collection) for as long
 * as the component stays mounted, and exposes a `publish` bound to it.
 * See `@cratebase/client`'s `Channel` for the full contract, including
 * how `_channels` config rows gate subscribe/publish. */

import { useCallback, useEffect, useRef, useState } from "react";
import type { Channel, ChannelMessage, CratebaseClient } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface UseChannelOptions {
  /** Set to `false` to skip subscribing entirely (e.g. before the caller
   * knows which channel to join). Defaults to `true`. */
  enabled?: boolean;
  /** Called for every message this channel delivers, including the
   * built-in `presence.join`/`.update`/`.leave` events — see
   * `usePresence` for a hook that already filters to just those. */
  onMessage?: (message: ChannelMessage) => void;
}

export interface UseChannelResult<T = unknown> {
  /** The most recently received message, or `null` before the first one
   * arrives. Prefer `options.onMessage` for anything that needs to react
   * to *every* message (a chat log, say) — this is a convenience for the
   * common "just show the latest" case (a live counter, a cursor
   * position) and skips any message that arrives before React re-renders. */
  lastMessage: ChannelMessage<T> | null;
  /** Whether the channel's subscribe/connect handshake has completed. A
   * subscribe that's silently unauthorized (the channel's `subscribeRule`
   * rejects this caller) looks identical to one that's simply quiet —
   * see `Channel.subscribe`'s doc comment — so this is "connected", not
   * "authorized". */
  connected: boolean;
  error: unknown;
  publish: (event: string, data?: T) => Promise<void>;
}

export function useChannel<T = unknown>(
  client: CratebaseClient<any>,
  name: string,
  options?: UseChannelOptions,
): UseChannelResult<T>;
export function useChannel<T = unknown>(name: string, options?: UseChannelOptions): UseChannelResult<T>;
export function useChannel<T = unknown>(
  arg0: CratebaseClient<any> | string,
  arg1?: string | UseChannelOptions,
  arg2?: UseChannelOptions,
): UseChannelResult<T> {
  const isClient = typeof arg0 !== "string";
  const explicitClient = isClient ? (arg0 as CratebaseClient<any>) : undefined;
  const name = (isClient ? (arg1 as string) : arg0)!;
  const options: UseChannelOptions = (isClient ? arg2 : (arg1 as UseChannelOptions | undefined)) ?? {};

  const client = useResolvedClient(explicitClient);
  const { enabled = true, onMessage } = options;
  const onMessageRef = useRef(onMessage);
  onMessageRef.current = onMessage;

  const [lastMessage, setLastMessage] = useState<ChannelMessage<T> | null>(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const channelRef = useRef<Channel<T> | null>(null);

  useEffect(() => {
    if (!enabled) {
      setConnected(false);
      return;
    }

    let cancelled = false;
    const channel = client.channel<T>(name);
    channelRef.current = channel;
    setError(null);

    let unsubscribe: (() => void) | undefined;
    channel
      .subscribe((message) => {
        setLastMessage(message);
        onMessageRef.current?.(message as ChannelMessage);
      })
      .then((unsub) => {
        if (cancelled) {
          unsub();
          return;
        }
        unsubscribe = unsub;
        setConnected(true);
      })
      .catch((err) => {
        if (!cancelled) setError(err);
      });

    return () => {
      cancelled = true;
      unsubscribe?.();
      channelRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, name, enabled]);

  const publish = useCallback(
    (event: string, data?: T) => {
      const channel = channelRef.current ?? client.channel<T>(name);
      return channel.publish(event, data);
    },
    [client, name],
  );

  return { lastMessage, connected, error, publish };
}
