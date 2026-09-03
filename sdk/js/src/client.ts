import { AuthStore } from "./auth-store.js";
import { ClientResponseError, type ApiErrorBody } from "./error.js";
import { AdminService } from "./admin-service.js";
import { RecordService } from "./record-service.js";
import { SchemaService } from "./schema-service.js";
import { RealtimeService } from "./realtime.js";

export interface SendOptions extends Omit<RequestInit, "body" | "headers"> {
  body?: Record<string, unknown> | FormData;
  headers?: Record<string, string>;
  query?: Record<string, string | number | boolean | undefined>;
}

/**
 * Cratebase API client. One instance per backend URL; safe to share as a
 * singleton across your app (records/collections/auth all read the
 * `authStore` lazily on every request, so logging in updates every
 * in-flight caller).
 *
 * ```ts
 * const cb = new Cratebase("https://api.example.com");
 * await cb.collection("users").authWithPassword("a@b.com", "secret");
 * const posts = await cb.collection("posts").getList(1, 20, { filter: "published = true" });
 * ```
 */
export class Cratebase {
  readonly baseUrl: string;
  readonly authStore: AuthStore;
  readonly admins: AdminService;
  readonly collections: SchemaService;
  readonly realtime: RealtimeService;

  constructor(baseUrl = "/", authStore: AuthStore = new AuthStore()) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.authStore = authStore;
    this.admins = new AdminService(this);
    this.collections = new SchemaService(this);
    this.realtime = new RealtimeService(this);
  }

  /** Get a `RecordService` bound to one collection (by id or name). */
  collection(idOrName: string): RecordService {
    return new RecordService(this, idOrName);
  }

  /** The download URL for a file field's stored filename. */
  getFileUrl(record: { collectionName?: string; collectionId?: string; id: string }, filename: string): string {
    const collection = record.collectionName || record.collectionId;
    return `${this.baseUrl}/api/files/${collection}/${record.id}/${filename}`;
  }

  /** Low-level request helper every service is built on. Throws
   * `ClientResponseError` for non-2xx responses. */
  async send<T>(path: string, options: SendOptions = {}): Promise<T> {
    const url = new URL(this.baseUrl + path, typeof window === "undefined" ? "http://localhost" : window.location.href);
    for (const [key, value] of Object.entries(options.query ?? {})) {
      if (value !== undefined) url.searchParams.set(key, String(value));
    }

    const headers: Record<string, string> = { ...options.headers };
    if (this.authStore.token) headers["authorization"] = `Bearer ${this.authStore.token}`;

    let body: BodyInit | undefined;
    if (options.body instanceof FormData) {
      body = options.body; // browser sets multipart boundary automatically
    } else if (options.body !== undefined) {
      headers["content-type"] = "application/json";
      body = JSON.stringify(options.body);
    }

    const response = await fetch(url.toString(), { ...options, headers, body });
    if (response.status === 204) return undefined as T;

    const text = await response.text();
    const data = text ? JSON.parse(text) : undefined;

    if (!response.ok) {
      throw new ClientResponseError(url.toString(), response.status, data as Partial<ApiErrorBody>);
    }
    return data as T;
  }
}
