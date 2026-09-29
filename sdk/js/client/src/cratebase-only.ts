/** Cratebase-only endpoints with no PocketBase equivalent: vector
 * search, the LLM chat gateway, MCP tool schemas, the durable job queue,
 * a client-side presence pattern, and custom SQL RPC. Written against
 * the minimal `Sender` shape rather than `Transport` directly, so
 * `@cratebase/extras` (which wraps the official `pocketbase` client's
 * `pb.send`) can reuse these implementations without a second copy. */

import type { RealtimeClient } from "./realtime.js";
import type { ListResult, RecordModel } from "./types.js";

export interface Sender {
  send<T>(path: string, options?: { method?: string; query?: Record<string, unknown>; body?: unknown }): Promise<T>;
}

/** A row `rpc()` returns: whatever columns the RPC's own `sql` selects,
 * shaped as a plain JSON object — no `RecordModel` bookkeeping, since a
 * `POST /api/rpc/{name}` result isn't a record of any collection. */
export type RpcRow = Record<string, unknown>;

/** `cb.rpc(name, params)`: call a custom-SQL RPC definition saved to the
 * `_rpc` collection (dashboard or `POST /api/collections/_rpc/records`).
 * `params` becomes the request body — bound by the server as real,
 * named SQL parameters against the definition's own declared `params`
 * schema, never string-interpolated — and the definition's `rule` is
 * evaluated against `@request.auth`/`@request.body` (`params`) before
 * anything runs. */
export async function rpc<T = RpcRow>(
  sender: Sender,
  name: string,
  params?: Record<string, unknown>,
): Promise<{ items: T[] }> {
  return sender.send<{ items: T[] }>(`/api/rpc/${encodeURIComponent(name)}`, {
    method: "POST",
    body: params ?? {},
  });
}

export interface NearestToOptions {
  limit?: number;
  filter?: string;
  fields?: string;
}

export async function nearestTo<T extends RecordModel = RecordModel>(
  sender: Sender,
  collection: string,
  field: string,
  to: number[] | string,
  options: NearestToOptions = {},
): Promise<ListResult<T>> {
  const target = Array.isArray(to) ? to.join(",") : to;
  return sender.send<ListResult<T>>(`/api/collections/${encodeURIComponent(collection)}/records`, {
    query: {
      nearestTo: `${field}:${target}`,
      nearestLimit: options.limit,
      filter: options.filter,
      fields: options.fields,
      skipTotal: true,
    },
  });
}

/** A `nearestTo` bound to `sender`, for `CratebaseClient#vector`. Every
 * other Cratebase-only namespace on the client (`llm`, `mcp`, `queue`,
 * `presence`) already closes over `this` so `cb.llm.chat(...)` etc. read
 * like ordinary bound methods; `vector.nearestTo` used to be a bare
 * re-export of the standalone `nearestTo(sender, collection, field, to,
 * options)` helper, which meant `cb.vector.nearestTo(...)` looked bound
 * but silently needed `cb` passed again as its first argument (or the
 * collection name would end up in the `sender` slot and break
 * confusingly) — live-verified building examples/team-board. This
 * returns a function overloaded to accept either the new bound call
 * `(collection, field, to, options)` or the old unbound call `(sender,
 * collection, field, to, options)`, so existing code keeps working. */
export function createNearestTo(sender: Sender) {
  function bound<T extends RecordModel = RecordModel>(
    collection: string,
    field: string,
    to: number[] | string,
    options?: NearestToOptions,
  ): Promise<ListResult<T>>;
  /**
   * @deprecated Pass `(collection, field, to, options)` instead —
   * `vector.nearestTo` is bound to the client now, so an explicit sender
   * is no longer needed. Kept for backwards compatibility with code
   * written against the old unbound helper.
   */
  function bound<T extends RecordModel = RecordModel>(
    explicitSender: Sender,
    collection: string,
    field: string,
    to: number[] | string,
    options?: NearestToOptions,
  ): Promise<ListResult<T>>;
  function bound<T extends RecordModel = RecordModel>(
    a: Sender | string,
    b: string,
    c: number[] | string,
    d?: NearestToOptions | number[] | string,
    e?: NearestToOptions,
  ): Promise<ListResult<T>> {
    if (typeof a === "string") {
      return nearestTo<T>(sender, a, b, c, d as NearestToOptions | undefined);
    }
    return nearestTo<T>(a, b, c as unknown as string, d as number[] | string, e);
  }
  return bound;
}

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

export interface ChatOptions {
  collection?: string;
  clientId?: string;
  onDelta?: (delta: string) => void;
  onError?: (message: string) => void;
}

export interface ChatResult {
  reply: string;
  promptTokens: number;
  completionTokens: number;
  record?: unknown;
}

export async function chat(
  sender: Sender,
  messages: ChatMessage[],
  realtime: RealtimeClient | undefined,
  options: ChatOptions = {},
): Promise<ChatResult> {
  const requestId = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  let unsubscribe: (() => void) | undefined;
  if (realtime && (options.onDelta || options.onError)) {
    unsubscribe = await realtime.subscribe("llm", requestId, (event) => {
      if (!event || typeof event !== "object") return;
      const requestIdField = "requestId" in event ? event.requestId : undefined;
      if (requestIdField !== requestId) return;
      const delta = "delta" in event ? event.delta : undefined;
      const message = "message" in event ? event.message : undefined;
      if (typeof delta === "string") options.onDelta?.(delta);
      if (typeof message === "string") options.onError?.(message);
    });
  }
  try {
    return await sender.send<ChatResult>("/api/llm/chat", {
      method: "POST",
      body: { messages, collection: options.collection, clientId: options.onDelta || options.onError ? requestId : undefined },
    });
  } finally {
    unsubscribe?.();
  }
}

export interface ToolSchema {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export async function toolSchema(sender: Sender, collection: string): Promise<ToolSchema> {
  return sender.send<ToolSchema>(`/api/collections/${encodeURIComponent(collection)}/tool-schema`);
}

export async function toolSchemas(sender: Sender, collections: string[]): Promise<ToolSchema[]> {
  return Promise.all(collections.map((c) => toolSchema(sender, c)));
}

export interface EnqueueOptions {
  /** How many attempts (including the first) before the job is given up
   * on for good and left `failed`. Server default: 5. */
  maxAttempts?: number;
  /** ISO-8601/RFC3339 timestamp; the job is not eligible to run before
   * this. Takes precedence over `delay` when both are given. */
  runAt?: string;
  /** @deprecated Use `runAt` — kept as an accepted alias since the server
   * still reads it under this name too. */
  runAfter?: string;
  /** Milliseconds from now to delay the job's first attempt. Ignored if
   * `runAt`/`runAfter` is also given. */
  delay?: number;
  /** A caller-chosen idempotency key: a second `enqueue` call with the
   * same non-empty key while an earlier job with it is still
   * pending/in-progress is a no-op — see `EnqueuedJob.deduped`. */
  dedupeKey?: string;
  /** Higher runs first among otherwise-due jobs. Server default: `0`. */
  priority?: number;
}

export interface EnqueuedJob {
  id: string;
  queue: string;
  status: string;
  runAfter: string;
  /** `true` when `options.dedupeKey` matched an already pending/
   * in-progress job — `id`/`status`/`runAfter` describe *that* job, not
   * a newly inserted one. */
  deduped: boolean;
}

/** `cb.queue.enqueue(queue, payload, options)` — `POST /api/queue/enqueue`
 * (the `POST /api/plugins/queue/enqueue` alias still works server-side,
 * but this SDK always calls the canonical route). Superuser or API key
 * — the same trust tier as `_cron_jobs`/`_webhooks`: `pb.authStore` needs
 * a `_superusers` session, or an API key header, before calling this.
 * Works even while `settings.queue.enabled` is off; the job just sits
 * `pending` until an operator turns processing on. */
export async function enqueue(
  sender: Sender,
  queue: string,
  payload: unknown,
  options: EnqueueOptions = {},
): Promise<EnqueuedJob> {
  return sender.send<EnqueuedJob>("/api/queue/enqueue", {
    method: "POST",
    body: {
      queue,
      payload,
      maxAttempts: options.maxAttempts,
      runAt: options.runAt,
      runAfter: options.runAfter,
      delay: options.delay,
      dedupeKey: options.dedupeKey,
      priority: options.priority,
    },
  });
}

/** `cb.queue.retry(id)` — `POST /api/queue/jobs/{id}/retry`: resets a
 * `failed` job back to `pending` with a fresh `attempts` budget. Rejects
 * (404) a missing job, or (400) one that isn't currently `failed`. */
export async function retryJob(sender: Sender, id: string): Promise<void> {
  await sender.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}/retry`, { method: "POST" });
}

/** `cb.queue.delete(id)` — `DELETE /api/queue/jobs/{id}`: removes a
 * `_queue_jobs` row outright, whatever its status. */
export async function deleteJob(sender: Sender, id: string): Promise<void> {
  await sender.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export interface MailRecipient {
  address: string;
  name?: string;
}

export type MailAddress = string | MailRecipient | Array<string | MailRecipient>;

export interface SendMailOptions {
  to: MailAddress;
  cc?: MailAddress;
  bcc?: MailAddress;
  template?: string;
  locale?: string;
  data?: Record<string, unknown>;
  subject?: string;
  html?: string;
  text?: string;
  from?: string | MailRecipient;
  replyTo?: string;
}

export interface SendMailResult {
  id: string;
  status: "queued" | "sent" | "failed";
  error?: string;
}

/** `POST /api/mails/send`.
 *
 * A superuser or API key may always send anything: a `template`, or raw
 * `subject`/`html`/`text`, with any `from`/`cc`/`bcc`/`replyTo` override.
 *
 * Any other caller — including an anonymous one — may call this too, but
 * only `to` + `template` (+ `data`/`locale`): raw content and every
 * override are refused outright. It's allowed only when that template's
 * `_emailTemplates.sendRule` is set (non-`null`) and evaluates `true` for
 * every `to` address, evaluated against `@request.auth.*` (the caller,
 * if any) and `@request.body.{to,data,locale}` — the same filter-rule
 * language a collection API rule uses. This is what lets a frontend call
 * `cb.mails.send({ template: "invite", to, data })` directly with no
 * backend of its own; see the email docs' "Frontend sends" section for
 * the security model and worked examples. A denied call is a `403` that
 * never distinguishes "no such template" from "the rule rejected you".
 *
 * Either way, `to`/`cc`/`bcc` together may not exceed 50 recipients for a
 * superuser/API key, or 5 for anyone else. */
export async function sendMail(sender: Sender, options: SendMailOptions): Promise<SendMailResult> {
  return sender.send<SendMailResult>("/api/mails/send", { method: "POST", body: options });
}

export type PreviewMailOptions = Omit<SendMailOptions, "to" | "cc" | "bcc" | "from" | "replyTo">;

export interface PreviewMailResult {
  subject: string;
  html: string;
  text?: string;
}

/** `POST /api/mails/preview` — renders a template (or raw content)
 * without sending or logging anything. Superuser/API key only, unlike
 * `sendMail` above — this is a dashboard/tooling affordance, not part of
 * the `sendRule`-gated frontend surface. */
export async function previewMail(sender: Sender, options: PreviewMailOptions): Promise<PreviewMailResult> {
  return sender.send<PreviewMailResult>("/api/mails/preview", { method: "POST", body: options });
}

/** A `_notifications` row (`crates/core/src/collection.rs`'s
 * `default_system_collections` comment on `notifications`), as returned
 * by `cb.notifications.list()`/`.markRead()`/a realtime subscription. */
export interface NotificationRecord extends RecordModel {
  type: string;
  title: string;
  body: string;
  data: unknown;
  link: string;
  /** Empty string until the recipient marks this notification read. */
  readAt: string;
}

export type NotificationChannel = "inapp" | "email" | "push";

export interface SendNotificationOptions {
  /** One recipient record id, or several. */
  to: string | string[];
  /** The recipient auth collection; defaults to `"users"`. */
  collection?: string;
  type: string;
  title: string;
  body: string;
  data?: unknown;
  link?: string;
  /** Defaults to every channel (`["inapp", "email", "push"]`). */
  channels?: NotificationChannel[];
}

export interface SendNotificationResult {
  sent: number;
  recipients: string[];
}

/** `cb.notifications.send(...)` — `POST /api/notifications/send`,
 * superuser/API-key only (an operator/integration action that can target
 * *any* record, same trust tier as `cb.push` — see
 * `crate::routes::notifications`'s module doc). Fans the notification out
 * across `options.channels` (in-app row + realtime, email via the
 * `notification` template, push via `_push_subscriptions`) — see
 * `crate::notify` (server crate) for the full per-channel contract. The
 * same pipeline runs server-side as `$notify.send` in a JS hook. */
export async function sendNotification(
  sender: Sender,
  options: SendNotificationOptions,
): Promise<SendNotificationResult> {
  return sender.send<SendNotificationResult>("/api/notifications/send", { method: "POST", body: options });
}

/** `cb.notifications.unreadCount()` — `GET /api/notifications/unread-count`,
 * a cheap index-backed `COUNT(*)` scoped to the caller's own recipient
 * rows (any authenticated record, not superuser-only). */
export async function unreadNotificationCount(sender: Sender): Promise<number> {
  const res = await sender.send<{ count: number }>("/api/notifications/unread-count");
  return res.count;
}

export interface MarkAllNotificationsReadResult {
  updated: number;
}

/** `cb.notifications.markAllRead()` — `POST /api/notifications/read-all`:
 * marks every one of the caller's own unread notifications read in one
 * call, rather than a `readAt` update per row. */
export async function markAllNotificationsRead(sender: Sender): Promise<MarkAllNotificationsReadResult> {
  return sender.send<MarkAllNotificationsReadResult>("/api/notifications/read-all", { method: "POST" });
}

// ---------------------------------------------------------------------
// Realtime channels + presence (`crates/server/src/realtime.rs`'s
// "Realtime channels + presence" section)
// ---------------------------------------------------------------------

/** One `{event, data}` message published on a channel — includes the
 * three well-known presence events (`"presence.join"`/`.update`/`.leave`)
 * a subscriber gets alongside whatever custom events a publisher sends;
 * `channel.presence.onChange` is a filtered, typed convenience over the
 * same stream for those three specifically. */
export interface ChannelMessage<T = unknown> {
  event: string;
  data: T;
}

/** `{id, collectionName}` for whoever was authenticated when they
 * published/tracked presence, or `null` when they were anonymous. */
export interface ChannelAuth {
  id: string;
  collectionName: string;
}

export interface PresenceMember<T = unknown> {
  clientId: string;
  state: T;
  auth: ChannelAuth | null;
}

export type PresenceEventKind = "join" | "update" | "leave";

export interface ChannelPresence<T = unknown> {
  /** `POST /api/realtime/channels/{name}/presence` — send/refresh this
   * connection's own presence state. Ties to the current
   * `GET /api/realtime` SSE stream (connecting first if necessary), so
   * the server can automatically remove it if that stream disconnects
   * without an explicit leave. Call again on a timer (a few times more
   * often than the server's presence TTL, currently 45s — see
   * `PRESENCE_TTL` in `crates/server/src/realtime.rs`) to stay listed;
   * `useChannel`/`usePresence` (`@cratebase/react`) do this for you. */
  track(state: T): Promise<void>;
  /** `GET /api/realtime/channels/{name}/presence` — the current member
   * list, in join order. */
  list(): Promise<PresenceMember<T>[]>;
  /** Fires on every `presence.join`/`.update`/`.leave` for this channel —
   * a filtered view over the same stream `channel.subscribe` sees.
   * Returns an unsubscribe function. */
  onChange(handler: (kind: PresenceEventKind, member: PresenceMember<T>) => void): Promise<() => void>;
}

export interface Channel<T = unknown> {
  readonly name: string;
  /** `POST /api/realtime/channels/{name}/publish` — broadcast one
   * `{event, data}` message to every current subscriber, local and
   * cross-node alike. Refused (403) unless the channel's `_channels` row
   * (`publishRule`) allows this caller — with no matching row, the
   * channel is disabled by default. Oversized `data` is refused with 413
   * rather than silently failing cross-node delivery. */
  publish(event: string, data?: T): Promise<void>;
  /** Subscribe to every message published on this channel (including
   * presence events — see `ChannelMessage`'s doc comment). Requires the
   * channel's `subscribeRule` to allow this caller; an unauthorized
   * subscribe silently receives nothing rather than throwing (re-checked
   * on every delivery, so a rule edit or login/logout takes effect on the
   * very next message). Returns an unsubscribe function. */
  subscribe(handler: (message: ChannelMessage<T>) => void): Promise<() => void>;
  presence: ChannelPresence<T>;
}

/** `cb.channel(name)` — see `Channel`'s own doc comments for what each
 * method does. Creating a `Channel` does no network I/O by itself; the
 * first `subscribe`/`publish`/`presence.*` call is what actually opens
 * the SSE connection or makes the HTTP request. */
export function createChannel<T = unknown>(sender: Sender, realtime: RealtimeClient, name: string): Channel<T> {
  const base = `/api/realtime/channels/${encodeURIComponent(name)}`;
  const topic = `channel:${name}`;

  const presence: ChannelPresence<T> = {
    async track(state) {
      const clientId = await realtime.ensureClientId();
      await sender.send<{ event: string }>(`${base}/presence`, {
        method: "POST",
        body: { clientId, state },
      });
    },
    async list() {
      const res = await sender.send<{ members: PresenceMember<T>[] }>(`${base}/presence`);
      return res.members;
    },
    async onChange(handler) {
      return realtime.subscribeTopic(topic, (raw) => {
        const msg = raw as ChannelMessage<PresenceMember<T>>;
        if (msg.event === "presence.join") handler("join", msg.data);
        else if (msg.event === "presence.update") handler("update", msg.data);
        else if (msg.event === "presence.leave") handler("leave", msg.data);
      });
    },
  };

  return {
    name,
    async publish(event, data) {
      await sender.send<void>(`${base}/publish`, {
        method: "POST",
        body: { event, data: data ?? null },
      });
    },
    async subscribe(handler) {
      return realtime.subscribeTopic(topic, (raw) => handler(raw as ChannelMessage<T>));
    },
    presence,
  };
}

/** Reads a magic-link token out of the current page's URL (or an
 * explicitly-passed one), matching the default
 * `authOptions.magicLink.urlTemplate` (`.../auth/magic-link?token=...`).
 * `null` outside a browser or when the param is absent — this never
 * throws, so it's safe to call unconditionally on every page load before
 * deciding whether to call `auth.signIn.magicLink`. */
export function getMagicLinkTokenFromUrl(url?: string | URL, param = "token"): string | null {
  try {
    const target = url ?? (typeof window === "undefined" ? undefined : window.location.href);
    if (!target) return null;
    const parsed = typeof target === "string" ? new URL(target) : target;
    return parsed.searchParams.get(param);
  } catch {
    return null;
  }
}

export interface PresenceOptions {
  heartbeatMs?: number;
  staleMs?: number;
  lastSeenField?: string;
}

export interface Presence {
  online: Set<string>;
  subscribe(callback: (online: Set<string>) => void): () => void;
  stop(): Promise<void>;
}

interface RecordService {
  update(id: string, data: Record<string, unknown>): Promise<RecordModel>;
  create(data: Record<string, unknown>): Promise<RecordModel>;
  fullList(): Promise<RecordModel[]>;
  subscribe(
    topic: string,
    handler: (event: { action: string; record: RecordModel }) => void,
  ): Promise<() => unknown>;
}

const DEFAULT_HEARTBEAT_MS = 20_000;
export async function trackPresence(
  collection: RecordService,
  data: Record<string, unknown> & { id?: string },
  options: PresenceOptions = {},
): Promise<Presence> {
  const heartbeatMs = options.heartbeatMs ?? DEFAULT_HEARTBEAT_MS;
  const staleMs = options.staleMs ?? heartbeatMs * 3;
  const lastSeenField = options.lastSeenField ?? "lastSeenAt";

  let id = data.id;
  const seed: Record<string, unknown> = { ...data, [lastSeenField]: new Date().toISOString() };
  if (id) {
    await collection.update(id, seed);
  } else {
    const created = await collection.create(seed);
    id = created.id;
  }

  const lastSeen = new Map<string, number>();
  const callbacks = new Set<(online: Set<string>) => void>();

  const computeOnline = (): Set<string> => {
    const now = Date.now();
    const online = new Set<string>();
    for (const [recordId, seenAt] of lastSeen) {
      if (now - seenAt <= staleMs) online.add(recordId);
    }
    return online;
  };
  const notify = () => {
    const online = computeOnline();
    for (const cb of callbacks) cb(online);
  };

  for (const row of await collection.fullList()) {
    const seenAt = row[lastSeenField];
    if (typeof seenAt === "string") lastSeen.set(row.id, new Date(seenAt).getTime());
  }
  lastSeen.set(id, Date.now());

  const unsubscribeRealtime = await collection.subscribe("*", (event) => {
    const seenAt = event.record[lastSeenField];
    if (event.action === "delete") {
      lastSeen.delete(event.record.id);
    } else if (typeof seenAt === "string") {
      lastSeen.set(event.record.id, new Date(seenAt).getTime());
    }
    notify();
  });

  const heartbeat = setInterval(() => {
    lastSeen.set(id!, Date.now());
    void collection.update(id!, { [lastSeenField]: new Date().toISOString() });
    notify();
  }, heartbeatMs);
  const sweep = setInterval(notify, Math.max(1000, Math.floor(staleMs / 4)));

  return {
    get online() {
      return computeOnline();
    },
    subscribe(callback) {
      callbacks.add(callback);
      callback(computeOnline());
      return () => callbacks.delete(callback);
    },
    async stop() {
      clearInterval(heartbeat);
      clearInterval(sweep);
      unsubscribeRealtime();
      callbacks.clear();
    },
  };
}
