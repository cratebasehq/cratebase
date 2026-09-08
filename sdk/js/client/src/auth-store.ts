/** Where the client keeps its bearer token + record between requests. */

import type { RecordModel } from "./types.js";

export type OnStoreChange = (token: string, record: RecordModel | null) => void;

export interface AuthStore {
  readonly token: string;
  readonly record: RecordModel | null;
  readonly isValid: boolean;
  readonly isSuperuser: boolean;
  save(token: string, record: RecordModel | null): void;
  clear(): void;
  onChange(callback: OnStoreChange, fireImmediately?: boolean): () => void;
}

function decodeExp(token: string): number | undefined {
  const parts = token.split(".");
  if (parts.length !== 3) return undefined;
  try {
    const payload = JSON.parse(atob(parts[1]!.replace(/-/g, "+").replace(/_/g, "/"))) as { exp?: number };
    return payload.exp;
  } catch {
    return undefined;
  }
}

abstract class BaseAuthStore implements AuthStore {
  protected currentToken = "";
  protected currentRecord: RecordModel | null = null;
  private listeners: Set<OnStoreChange> = new Set();

  get token(): string {
    return this.currentToken;
  }

  get record(): RecordModel | null {
    return this.currentRecord;
  }

  get isValid(): boolean {
    if (!this.currentToken) return false;
    const exp = decodeExp(this.currentToken);
    return exp === undefined || exp * 1000 > Date.now();
  }

  get isSuperuser(): boolean {
    return this.currentRecord?.collectionName === "_superusers";
  }

  save(token: string, record: RecordModel | null): void {
    this.currentToken = token;
    this.currentRecord = record;
    this.persist();
    this.notify();
  }

  clear(): void {
    this.currentToken = "";
    this.currentRecord = null;
    this.persist();
    this.notify();
  }

  onChange(callback: OnStoreChange, fireImmediately = false): () => void {
    this.listeners.add(callback);
    if (fireImmediately) callback(this.currentToken, this.currentRecord);
    return () => this.listeners.delete(callback);
  }

  protected notify(): void {
    for (const listener of this.listeners) listener(this.currentToken, this.currentRecord);
  }

  /** Persist to whatever backing store a subclass uses; a no-op for
   * `MemoryAuthStore`. */
  protected persist(): void {}
}

/** In-memory only — the default outside a browser, and always what
 * `client.auth.admin.impersonate()` returns so an impersonated session
 * never touches the caller's own persisted store. */
export class MemoryAuthStore extends BaseAuthStore {}

/** Browser `localStorage`-backed, JSON-encoded `{token, record}` under
 * `storageKey`. The default in a browser. */
export class LocalAuthStore extends BaseAuthStore {
  private readonly storageKey: string;

  constructor(storageKey = "cratebase_auth") {
    super();
    this.storageKey = storageKey;
    this.hydrate();
  }

  private hydrate(): void {
    try {
      const raw = globalThis.localStorage?.getItem(this.storageKey);
      if (!raw) return;
      const parsed = JSON.parse(raw) as { token?: string; record?: RecordModel | null };
      this.currentToken = parsed.token ?? "";
      this.currentRecord = parsed.record ?? null;
    } catch {
      // Corrupt/foreign value under this key — start anonymous.
    }
  }

  protected override persist(): void {
    try {
      globalThis.localStorage?.setItem(
        this.storageKey,
        JSON.stringify({ token: this.currentToken, record: this.currentRecord }),
      );
    } catch {
      // Storage unavailable (private browsing quota, SSR) — in-memory
      // state still works for the lifetime of this store instance.
    }
  }
}

export interface AsyncAuthStoreOptions {
  save: (serialized: string) => Promise<void> | void;
  clear?: () => Promise<void> | void;
  initial?: Promise<string | null> | string | null;
}

/** For React Native / Expo and any other host whose persistence API is
 * itself async (`AsyncStorage.setItem` returns a `Promise`). */
export class AsyncAuthStore extends BaseAuthStore {
  private readonly saveFn: (serialized: string) => Promise<void> | void;
  private readonly clearFn: (() => Promise<void> | void) | undefined;
  private ready: Promise<void>;

  constructor(options: AsyncAuthStoreOptions) {
    super();
    this.saveFn = options.save;
    this.clearFn = options.clear;
    this.ready = Promise.resolve(options.initial).then((raw) => {
      if (!raw) return;
      try {
        const parsed = JSON.parse(raw) as { token?: string; record?: RecordModel | null };
        this.currentToken = parsed.token ?? "";
        this.currentRecord = parsed.record ?? null;
        this.notify();
      } catch {
        // Corrupt initial value — start anonymous.
      }
    });
  }

  /** Resolves once the constructor's `initial` value has hydrated the
   * store — a caller building UI on `isValid`/`record` should await this
   * before its first render. */
  async whenReady(): Promise<void> {
    await this.ready;
  }

  protected override persist(): void {
    if (this.currentToken) {
      void this.saveFn(JSON.stringify({ token: this.currentToken, record: this.currentRecord }));
    } else {
      void (this.clearFn ? this.clearFn() : this.saveFn(JSON.stringify({ token: "", record: null })));
    }
  }
}

export function defaultAuthStore(): AuthStore {
  if (typeof globalThis.localStorage !== "undefined") return new LocalAuthStore();
  return new MemoryAuthStore();
}
