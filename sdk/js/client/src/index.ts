/** `@cratebase/client` — the first-party typed TypeScript SDK: records,
 * auth (password/OTP/OAuth2/MFA, cookie sessions, impersonation, bans),
 * realtime, files, batch, and admin APIs. Wire-compatible with the
 * official `pocketbase` npm SDK's server surface; `@cratebase/extras`
 * remains available for projects staying on that SDK. */

import type { AuthStore } from "./auth-store.js";
import { defaultAuthStore, MemoryAuthStore } from "./auth-store.js";
import { AuthNamespace } from "./auth.js";
import { AdminNamespace } from "./admin.js";
import { BatchBuilder } from "./batch.js";
import * as cratebaseOnly from "./cratebase-only.js";
import { FilesService } from "./files.js";
import { CollectionService } from "./records.js";
import { RealtimeClient } from "./realtime.js";
import { Transport } from "./transport.js";
import type { RecordModel } from "./types.js";

export { CratebaseError } from "./transport.js";
export type { QueryValue, SendOptions } from "./transport.js";
export { MemoryAuthStore, LocalAuthStore, AsyncAuthStore, defaultAuthStore } from "./auth-store.js";
export type { AuthStore, OnStoreChange, AsyncAuthStoreOptions } from "./auth-store.js";
export { AuthNamespace } from "./auth.js";
export type {
  AuthResult,
  AuthMethodsList,
  SessionRow,
  SignInPasswordOptions,
  SignInOtpOptions,
  SignInCodeOptions,
  SignInSocialOptions,
  SocialMode,
} from "./auth.js";
export { CollectionService } from "./records.js";
export { RealtimeClient } from "./realtime.js";
export { FilesService } from "./files.js";
export type { FileURLOptions } from "./files.js";
export { BatchBuilder } from "./batch.js";
export type { BatchResult } from "./batch.js";
export { AdminNamespace } from "./admin.js";
export type { LogEntry } from "./admin.js";
export { filter, raw, Raw } from "./filter.js";
export type {
  RecordModel,
  CollectionModel,
  CollectionField,
  CollectionInput,
  CollectionFieldInput,
  ListResult,
  ListOptions,
  ViewOptions,
  WriteOptions,
  SubscribeOptions,
  RecordSubscription,
  SortSpec,
  OAuth2Provider,
} from "./types.js";
export const vector = { nearestTo: cratebaseOnly.nearestTo };
export type { NearestToOptions } from "./cratebase-only.js";
export type { ChatMessage, ChatOptions, ChatResult, ToolSchema, EnqueueOptions, EnqueuedJob, PresenceOptions, Presence, Sender } from "./cratebase-only.js";

export interface CreateClientOptions {
  authStore?: AuthStore;
  authCollection?: string;
  credentials?: RequestCredentials;
  cookie?: string;
  fetch?: typeof fetch;
  headers?: Record<string, string>;
  lang?: string;
}

/** Every non-system collection a codegen'd schema carries — pass
 * `Schema` from `@cratebase/schema-codegen`'s output to `createClient`
 * for typed `client.collection(name)`. The constraint is deliberately
 * `Record<string, Record<string, unknown>>`, not `Record<string,
 * RecordModel>`: a generated interface only carries the collection's own
 * fields, and every read still widens to `T & RecordModel`. */
export type AnySchema = Record<string, Record<string, unknown>>;

export class CratebaseClient<S extends AnySchema = Record<string, RecordModel>> {
  readonly transport: Transport;
  readonly realtime: RealtimeClient;
  readonly files: FilesService;
  readonly admin: AdminNamespace;
  readonly auth: AuthNamespace;
  readonly vector = vector;
  private readonly authCollection: string;
  private readonly collections: Map<string, CollectionService<Record<string, unknown>>> = new Map();
  private readonly authNamespaces: Map<string, AuthNamespace> = new Map();

  constructor(baseUrl: string, options: CreateClientOptions = {}) {
    this.transport = new Transport(baseUrl, {
      fetch: options.fetch,
      cookie: options.cookie,
      headers: options.headers,
      credentials: options.credentials ?? "same-origin",
      lang: options.lang,
    });
    this.realtime = new RealtimeClient(this.transport, () => this.authHeaderFor(this.authCollection));
    this.files = new FilesService(this.transport, () => this.authHeaderFor(this.authCollection));
    this.admin = new AdminNamespace(this.transport, () => this.authHeaderFor(this.authCollection));
    this.authCollection = options.authCollection ?? "users";

    const rootStore = options.authStore ?? defaultAuthStore();
    this.authNamespaces.set(this.authCollection, this.buildAuth(this.authCollection, rootStore));
    this.auth = this.authNamespaces.get(this.authCollection)!;
  }

  private authHeaderFor(collectionName: string): string | undefined {
    const ns = this.authNamespaces.get(collectionName);
    return ns?.token ? `Bearer ${ns.token}` : undefined;
  }

  private buildAuth(collectionName: string, store: AuthStore): AuthNamespace {
    const makeAuth = (name: string, s?: AuthStore): AuthNamespace => {
      const existing = !s && this.authNamespaces.get(name);
      if (existing) return existing;
      const ns = this.buildAuth(name, s ?? new MemoryAuthStore());
      if (!s) this.authNamespaces.set(name, ns);
      return ns;
    };
    return new AuthNamespace(this.transport, collectionName, store, makeAuth);
  }

  collection<K extends keyof S & string>(name: K): CollectionService<S[K]>;
  collection(name: string): CollectionService<RecordModel>;
  collection(name: string): CollectionService<Record<string, unknown>> {
    let service = this.collections.get(name);
    if (!service) {
      service = new CollectionService(this.transport, () => this.authHeaderFor(this.authCollection), this.realtime, name);
      this.collections.set(name, service);
    }
    return service;
  }

  batch(): BatchBuilder {
    return new BatchBuilder(this.transport, () => this.authHeaderFor(this.authCollection));
  }

  send<T>(path: string, options: Parameters<Transport["send"]>[1] = {}): Promise<T> {
    return this.transport.send<T>(path, options, () => this.authHeaderFor(this.authCollection));
  }

  buildURL(path: string): string {
    return this.transport.buildURL(path);
  }

  readonly llm = {
    chat: (messages: cratebaseOnly.ChatMessage[], options: cratebaseOnly.ChatOptions = {}) =>
      cratebaseOnly.chat(this, messages, this.realtime, options),
  };

  readonly mcp = {
    toolSchema: (collection: string) => cratebaseOnly.toolSchema(this, collection),
    toolSchemas: (collections: string[]) => cratebaseOnly.toolSchemas(this, collections),
  };

  readonly queue = {
    enqueue: (queue: string, payload: unknown, options: cratebaseOnly.EnqueueOptions = {}) =>
      cratebaseOnly.enqueue(this, queue, payload, options),
  };

  readonly presence = {
    track: (collectionName: string, data: Record<string, unknown> & { id?: string }, options: cratebaseOnly.PresenceOptions = {}) =>
      cratebaseOnly.trackPresence(this.collection(collectionName) as unknown as Parameters<typeof cratebaseOnly.trackPresence>[0], data, options),
  };
}

export function createClient<S extends AnySchema = Record<string, RecordModel>>(
  baseUrl: string,
  options: CreateClientOptions = {},
): CratebaseClient<S> {
  return new CratebaseClient<S>(baseUrl, options);
}
