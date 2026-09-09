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

export class RealtimeClient {
  private readonly transport: Transport;
  private readonly authHeader: () => string | undefined;
  private clientId: string | undefined;
  private listeners: Map<string, Set<Listener>> = new Map();
  private abort: AbortController | undefined;
  private connectPromise: Promise<void> | undefined;
  private reconnectDelay = RECONNECT_MIN_MS;
  private stopped = false;

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
    const key = topicKey(collection, topic, options);
    let set = this.listeners.get(key);
    if (!set) {
      set = new Set();
      this.listeners.set(key, set);
    }
    const listener: Listener = { topic: key, handler };
    set.add(listener);
    await this.ensureConnected();
    await this.syncSubscriptions();
    return async () => {
      set?.delete(listener);
      if (set && set.size === 0) this.listeners.delete(key);
      if (this.clientId) await this.syncSubscriptions();
    };
  }

  async stop(): Promise<void> {
    this.stopped = true;
    this.abort?.abort();
    this.listeners.clear();
    this.clientId = undefined;
  }

  private async ensureConnected(): Promise<void> {
    if (this.clientId) return;
    if (!this.connectPromise) this.connectPromise = this.connect();
    await this.connectPromise;
  }

  private async syncSubscriptions(): Promise<void> {
    if (!this.clientId) return;
    const subscriptions = [...this.listeners.keys()];
    await this.transport.send<void>(
      "/api/realtime",
      { method: "POST", body: { clientId: this.clientId, subscriptions } },
      this.authHeader,
    );
  }

  private async connect(): Promise<void> {
    this.stopped = false;
    this.abort = new AbortController();
    const headers: Record<string, string> = {};
    const token = this.authHeader();
    if (token) headers["Authorization"] = token;

    const res = await fetch(this.transport.buildURL("/api/realtime"), {
      headers,
      signal: this.abort.signal,
      credentials: this.transport.credentials,
    });
    if (!res.ok || !res.body) {
      throw new Error(`realtime connection failed with status ${res.status}`);
    }

    const clientIdPromise = new Promise<string>((resolve) => {
      this.readFrames(res.body!, (event, data) => {
        if (event === "PB_CONNECT") {
          const parsed = JSON.parse(data) as { clientId: string };
          this.clientId = parsed.clientId;
          this.reconnectDelay = RECONNECT_MIN_MS;
          resolve(parsed.clientId);
          return;
        }
        this.dispatch(event, data);
      }).finally(() => {
        this.clientId = undefined;
        this.connectPromise = undefined;
        if (!this.stopped) this.scheduleReconnect();
      });
    });
    await clientIdPromise;
  }

  private scheduleReconnect(): void {
    const delay = this.reconnectDelay;
    this.reconnectDelay = Math.min(this.reconnectDelay * 2, RECONNECT_MAX_MS);
    setTimeout(() => {
      if (this.stopped || this.listeners.size === 0) return;
      this.connectPromise = this.connect().then(() => this.syncSubscriptions());
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
