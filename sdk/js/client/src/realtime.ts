/** `GET /api/realtime` (SSE) + `POST /api/realtime` (subscriptions): one
 * shared connection per client, consumed with `fetch` and a
 * `ReadableStream` line parser rather than `EventSource`.
 *
 * `EventSource` cannot send an `Authorization` header and needs a
 * polyfill outside a browser — using `fetch` instead makes the stream
 * itself authenticated (bearer or cookie, whichever the client uses) and
 * removes the polyfill requirement entirely. */

import type { Transport } from "./transport.js";
import type { SubscribeOptions } from "./types.js";

interface Listener {
  topic: string;
  handler: (event: unknown) => void;
}

const RECONNECT_MIN_MS = 500;
const RECONNECT_MAX_MS = 15_000;

function topicKey(collection: string, topic: string, options: SubscribeOptions): string {
  if (Object.keys(options).length === 0) return `${collection}/${topic}`;
  const query = new URLSearchParams({ query: JSON.stringify(options) });
  return `${collection}/${topic}?options=${encodeURIComponent(query.toString())}`;
}

/** Connection lifecycle: `idle` (never started / dropped), `connecting`
 * (stream fetch in flight, waiting for `CB_CONNECT`), `open`, `backoff`
 * (a reconnect timer is pending), `stopped` (`stop()` was called; a later
 * subscribe restarts). */
type ConnState = "idle" | "connecting" | "open" | "backoff" | "stopped";

export class RealtimeClient {
  private readonly transport: Transport;
  private readonly authHeader: () => string | undefined;
  private clientId: string | undefined;
  private listeners: Map<string, Set<Listener>> = new Map();
  private abort: AbortController | undefined;
  private connectPromise: Promise<void> | undefined;
  private reconnectDelay = RECONNECT_MIN_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private state: ConnState = "idle";
  /** Bumped on every connect/stop/reconnect so a stale stream's teardown
   * can't clobber the state of a newer connection. */
  private generation = 0;
  private syncRun: Promise<void> | undefined;
  private syncDirty = false;

  constructor(transport: Transport, authHeader: () => string | undefined) {
    this.transport = transport;
    this.authHeader = authHeader;
  }

  async subscribe(
    collection: string,
    topic: string,
    handler: (event: unknown) => void,
    options: SubscribeOptions = {},
  ): Promise<() => void> {
    return this.subscribeTopic(topicKey(collection, topic, options), handler);
  }

  /** `subscribe`, minus the `"${collection}/${topic}"` key-building —
   * used directly by `cb.channel(name)` (`cratebase-only.ts`'s
   * `createChannel`), whose topic (`channel:<name>`) is already a
   * complete, flat string rather than a collection/topic pair. */
  async subscribeTopic(key: string, handler: (event: unknown) => void): Promise<() => void> {
    let set = this.listeners.get(key);
    if (!set) {
      set = new Set();
      this.listeners.set(key, set);
    }
    const mine = set;
    const listener: Listener = { topic: key, handler };
    mine.add(listener);
    const remove = (): void => {
      mine.delete(listener);
      // Only drop the map entry if it is still *our* set: after this set
      // emptied and was removed, a newer subscribe may have registered a
      // fresh set under the same key.
      if (mine.size === 0 && this.listeners.get(key) === mine) {
        this.listeners.delete(key);
      }
    };
    try {
      await this.ensureConnected();
      await this.syncSubscriptions();
    } catch (err) {
      remove();
      throw err;
    }
    return async () => {
      remove();
      if (this.clientId) await this.syncSubscriptions();
    };
  }

  /** This connection's `GET /api/realtime` client id, connecting first if
   * necessary. Presence tracking needs this: the server ties a presence
   * heartbeat to a live SSE stream so it can auto-leave on disconnect
   * (`crates/server/src/realtime.rs`'s `channel_presence_track`), so
   * `cb.channel(name).presence.track(...)` calls this before its own
   * `POST .../presence` rather than requiring the caller to already be
   * subscribed to something first. */
  async ensureClientId(): Promise<string> {
    await this.ensureConnected();
    if (!this.clientId) throw new Error("realtime client id is not available");
    return this.clientId;
  }

  async stop(): Promise<void> {
    this.state = "stopped";
    this.generation++;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.abort?.abort();
    this.abort = undefined;
    this.listeners.clear();
    this.clientId = undefined;
    this.connectPromise = undefined;
    this.reconnectDelay = RECONNECT_MIN_MS;
  }

  /** Call when the auth token changed (sign in / out / refresh): the SSE
   * stream was opened with the old credentials, so drop it and reconnect
   * (re-syncing every subscription) under the new ones. A no-op unless a
   * stream is open and something is subscribed. */
  reconnect(): void {
    if (this.state !== "open" || this.listeners.size === 0) return;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.reconnectDelay = RECONNECT_MIN_MS;
    this.generation++;
    this.abort?.abort();
    this.abort = undefined;
    this.clientId = undefined;
    this.connectPromise = undefined;
    this.state = "idle";
    this.scheduleReconnect(0);
  }

  private async ensureConnected(): Promise<void> {
    if (this.clientId) return;
    if (!this.connectPromise) {
      const p = this.connect();
      this.connectPromise = p;
      // A failed attempt must not stay cached: later callers retry.
      p.catch(() => {
        if (this.connectPromise === p) this.connectPromise = undefined;
      });
    }
    await this.connectPromise;
  }

  /** Serialised + coalesced: at most one POST in flight; changes made
   * meanwhile trigger exactly one more POST with the latest key set, so a
   * slow earlier request can never overwrite a newer subscription list. */
  private syncSubscriptions(): Promise<void> {
    if (!this.clientId) return Promise.resolve();
    if (this.syncRun) {
      this.syncDirty = true;
      return this.syncRun;
    }
    const run = (async () => {
      await Promise.resolve();
      try {
        do {
          this.syncDirty = false;
          if (!this.clientId) return;
          const subscriptions = [...this.listeners.keys()];
          await this.transport.send<void>(
            "/api/realtime",
            { method: "POST", body: { clientId: this.clientId, subscriptions } },
            this.authHeader,
          );
        } while (this.syncDirty);
      } finally {
        this.syncRun = undefined;
        this.syncDirty = false;
      }
    })();
    this.syncRun = run;
    return run;
  }

  private async connect(): Promise<void> {
    const gen = ++this.generation;
    this.state = "connecting";
    const abort = new AbortController();
    this.abort = abort;
    const headers: Record<string, string> = {};
    const token = this.authHeader();
    if (token) headers["Authorization"] = token;

    const fail = (): void => {
      if (this.generation === gen) this.state = "idle";
    };

    let res: Response;
    try {
      res = await fetch(this.transport.buildURL("/api/realtime"), {
        headers,
        signal: abort.signal,
        credentials: this.transport.credentials,
      });
    } catch (err) {
      fail();
      throw err;
    }
    if (!res.ok || !res.body) {
      fail();
      throw new Error(`realtime connection failed with status ${res.status}`);
    }
    if (this.generation !== gen) {
      // stop()/reconnect() ran while the fetch was in flight.
      abort.abort();
      throw new Error("realtime connection superseded");
    }

    await new Promise<string>((resolve, reject) => {
      this.readFrames(res.body!, (event, data) => {
        if (this.generation !== gen) return;
        if (event === "CB_CONNECT") {
          const parsed = JSON.parse(data) as { clientId: string };
          this.clientId = parsed.clientId;
          this.state = "open";
          this.reconnectDelay = RECONNECT_MIN_MS;
          resolve(parsed.clientId);
          return;
        }
        this.dispatch(event, data);
      })
        .catch(() => {
          // Network error / abort: handled the same as a clean close.
        })
        .then(() => {
          // No-op if CB_CONNECT already resolved; otherwise unblocks
          // everyone waiting on this connection attempt.
          reject(new Error("realtime stream closed before CB_CONNECT"));
          if (this.generation !== gen) return;
          this.clientId = undefined;
          this.connectPromise = undefined;
          this.state = "idle";
          if (this.listeners.size > 0) this.scheduleReconnect();
        });
    });
  }

  private scheduleReconnect(delayOverride?: number): void {
    if (this.reconnectTimer || this.state === "stopped") return;
    let delay: number;
    if (delayOverride !== undefined) {
      delay = delayOverride;
    } else {
      delay = this.reconnectDelay;
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, RECONNECT_MAX_MS);
    }
    this.state = "backoff";
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      if (this.state !== "backoff") return;
      if (this.listeners.size === 0) {
        this.state = "idle";
        return;
      }
      this.ensureConnected()
        .then(() => this.syncSubscriptions())
        .catch(() => {
          // Attempt failed (or the sync did): back off and retry.
          if (this.reconnectTimer || this.state === "stopped" || this.state === "open") return;
          this.state = "idle";
          if (this.listeners.size > 0) this.scheduleReconnect();
        });
    }, delay);
  }

  private dispatch(event: string, data: string): void {
    const set = this.listeners.get(event);
    if (!set || set.size === 0) return;
    let parsed: unknown;
    try {
      parsed = JSON.parse(data);
    } catch {
      return;
    }
    for (const listener of set) listener.handler(parsed);
  }

  /** Parses the SSE `event:`/`data:`/`id:` frame format
   * (`crates/server/src/realtime.rs`), calling `onFrame(event, data)`
   * once per complete frame (a blank line terminates one). */
  private async readFrames(
    body: ReadableStream<Uint8Array>,
    onFrame: (event: string, data: string) => void,
  ): Promise<void> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let currentEvent = "message";
    let currentData: string[] = [];

    const flush = () => {
      if (currentData.length > 0) onFrame(currentEvent, currentData.join("\n"));
      currentEvent = "message";
      currentData = [];
    };

    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let newlineIndex: number;
        while ((newlineIndex = buffer.indexOf("\n")) !== -1) {
          const rawLine = buffer.slice(0, newlineIndex);
          buffer = buffer.slice(newlineIndex + 1);
          const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
          if (line === "") {
            flush();
            continue;
          }
          if (line.startsWith("event:")) {
            currentEvent = line.slice(6).trim();
          } else if (line.startsWith("data:")) {
            currentData.push(line.slice(5).trim());
          }
          // `id:` lines are intentionally ignored — this client re-syncs
          // subscriptions on every reconnect rather than resuming from a
          // last-seen event id.
        }
      }
    } finally {
      reader.releaseLock();
    }
  }
}
