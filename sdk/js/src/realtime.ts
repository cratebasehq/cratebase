import type { Cratebase } from "./client.js";
import type { RecordModel } from "./types.js";

export interface RealtimeEvent<T extends RecordModel = RecordModel> {
  action: "create" | "update" | "delete";
  record: T;
}

export type RealtimeCallback<T extends RecordModel = RecordModel> = (event: RealtimeEvent<T>) => void;

function isConnectPayload(value: unknown): value is { clientId: string } {
  return typeof value === "object" && value !== null && "clientId" in value && typeof value.clientId === "string";
}

function isRealtimeEvent(value: unknown): value is RealtimeEvent {
  if (typeof value !== "object" || value === null || !("action" in value) || !("record" in value)) return false;
  const record = value.record;
  return typeof record === "object" && record !== null && "collectionName" in record && "id" in record;
}

/**
 * Live record change subscriptions over Server-Sent Events. Subscribe to a
 * bare collection name for every change, or `"collection/recordId"` for
 * just one record.
 *
 * ```ts
 * const unsubscribe = await cb.realtime.subscribe("posts", (e) => console.log(e.action, e.record));
 * // later
 * unsubscribe();
 * ```
 *
 * Requires a global `EventSource` (every browser; on Node.js install the
 * `eventsource` package and assign it to `globalThis.EventSource` first).
 */
export class RealtimeService {
  private eventSource: EventSource | null = null;
  private clientId = "";
  private readonly subscriptions = new Map<string, Set<RealtimeCallback>>();

  constructor(private readonly client: Cratebase) {}

  async subscribe<T extends RecordModel = RecordModel>(topic: string, callback: RealtimeCallback<T>): Promise<() => void> {
    if (!this.subscriptions.has(topic)) this.subscriptions.set(topic, new Set());
    this.subscriptions.get(topic)!.add(callback as RealtimeCallback);
    await this.ensureConnected();
    await this.syncSubscriptions();
    return () => void this.unsubscribe(topic, callback as RealtimeCallback);
  }

  async unsubscribe(topic: string, callback?: RealtimeCallback): Promise<void> {
    const set = this.subscriptions.get(topic);
    if (!set) return;
    if (callback) set.delete(callback);
    else set.clear();
    if (set.size === 0) this.subscriptions.delete(topic);

    if (this.subscriptions.size === 0) {
      this.disconnect();
    } else {
      await this.syncSubscriptions();
    }
  }

  disconnect(): void {
    this.eventSource?.close();
    this.eventSource = null;
    this.clientId = "";
  }

  private ensureConnected(): Promise<void> {
    if (this.eventSource) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const es = new EventSource(`${this.client.baseUrl}/api/realtime`);
      this.eventSource = es;

      // The DOM lib types `addEventListener` for custom SSE event names as
      // plain `Event`, but EventSource always dispatches `MessageEvent` —
      // this is a library typing gap, not untrusted-shape guessing.
      es.addEventListener("PB_CONNECT", (event) => {
        const messageEvent = event as MessageEvent;
        const parsed: unknown = JSON.parse(messageEvent.data);
        if (!isConnectPayload(parsed)) return;
        // The browser's EventSource auto-reconnects on transient drops
        // (network blip, server restart) and the server issues a fresh
        // clientId on every reconnect. The server-side subscription list
        // is keyed by clientId, so without re-syncing here a reconnected
        // client goes silently deaf: the SSE stream looks "connected" but
        // never receives another event for topics it was already
        // subscribed to.
        const isReconnect = this.clientId !== "" && this.clientId !== parsed.clientId;
        this.clientId = parsed.clientId;
        if (isReconnect) void this.syncSubscriptions();
        resolve();
      });

      es.addEventListener("message", (event) => {
        const messageEvent = event as MessageEvent;
        const parsed: unknown = JSON.parse(messageEvent.data);
        if (!isRealtimeEvent(parsed)) return;
        const topics = [parsed.record.collectionName, `${parsed.record.collectionName}/${parsed.record.id}`];
        for (const topic of topics) {
          this.subscriptions.get(topic)?.forEach((cb) => cb(parsed));
        }
      });

      es.onerror = () => {
        if (!this.clientId) reject(new Error("failed to connect to /api/realtime"));
        // The browser's EventSource auto-reconnects on transient errors on
        // its own; the PB_CONNECT handler above resyncs subscriptions once
        // the fresh clientId arrives, so there's nothing to do here beyond
        // rejecting a first-connect failure.
      };
    });
  }

  private async syncSubscriptions(): Promise<void> {
    if (!this.clientId) return;
    await this.client.send("/api/realtime", {
      method: "POST",
      body: { clientId: this.clientId, subscriptions: [...this.subscriptions.keys()] },
    });
  }
}
