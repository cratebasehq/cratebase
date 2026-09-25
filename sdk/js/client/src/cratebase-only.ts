/** Cratebase-only endpoints with no PocketBase equivalent: vector
 * search, the LLM chat gateway, MCP tool schemas, the durable job queue,
 * and a client-side presence pattern. Written against the minimal
 * `Sender` shape rather than `Transport` directly, so `@cratebase/extras`
 * (which wraps the official `pocketbase` client's `pb.send`) can reuse
 * these implementations without a second copy. */

import type { RealtimeClient } from "./realtime.js";
import type { ListResult, RecordModel } from "./types.js";

export interface Sender {
  send<T>(path: string, options?: { method?: string; query?: Record<string, unknown>; body?: unknown }): Promise<T>;
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
  maxAttempts?: number;
  runAfter?: string;
}

export interface EnqueuedJob {
  id: string;
  queue: string;
  status: string;
  runAfter?: string;
}

export async function enqueue(
  sender: Sender,
  queue: string,
  payload: unknown,
  options: EnqueueOptions = {},
): Promise<EnqueuedJob> {
  return sender.send<EnqueuedJob>("/api/plugins/queue/enqueue", {
    method: "POST",
    body: { queue, payload, maxAttempts: options.maxAttempts, runAfter: options.runAfter },
  });
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
