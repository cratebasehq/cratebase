export type AuthModel = Record<string, unknown> | null;

type Listener = (token: string, model: AuthModel) => void;

/** Holds the current auth token + record/admin, persisted to
 * `localStorage` when available (browser) so a page reload doesn't log the
 * user out. In non-browser environments (SSR, React Native without a
 * storage polyfill, CLI scripts) it just holds state in memory for the
 * process lifetime — set your own `save`/`clear` if you need custom
 * persistence (e.g. `AsyncStorage` on React Native, an httpOnly cookie on
 * the server). */
export class AuthStore {
  private storageKey: string;
  private listeners: Listener[] = [];
  private _token = "";
  private _model: AuthModel = null;

  constructor(storageKey = "cratebase_auth") {
    this.storageKey = storageKey;
    this.loadFromStorage();
  }

  get token(): string {
    return this._token;
  }

  get model(): AuthModel {
    return this._model;
  }

  get isValid(): boolean {
    if (!this._token) return false;
    const payload = decodeJwtPayload(this._token);
    if (!payload || typeof payload.exp !== "number") return true;
    return payload.exp * 1000 > Date.now();
  }

  save(token: string, model: AuthModel): void {
    this._token = token;
    this._model = model;
    this.persist();
    this.emit();
  }

  clear(): void {
    this._token = "";
    this._model = null;
    this.persist();
    this.emit();
  }

  onChange(listener: Listener): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }

  private emit(): void {
    for (const listener of this.listeners) listener(this._token, this._model);
  }

  private persist(): void {
    const storage = getLocalStorage();
    if (!storage) return;
    if (this._token) {
      storage.setItem(this.storageKey, JSON.stringify({ token: this._token, model: this._model }));
    } else {
      storage.removeItem(this.storageKey);
    }
  }

  private loadFromStorage(): void {
    const storage = getLocalStorage();
    if (!storage) return;
    const raw = storage.getItem(this.storageKey);
    if (!raw) return;
    try {
      const parsed = JSON.parse(raw) as { token: string; model: AuthModel };
      this._token = parsed.token ?? "";
      this._model = parsed.model ?? null;
    } catch {
      // corrupt storage entry: ignore, start logged out
    }
  }
}

function getLocalStorage(): Storage | null {
  try {
    return typeof window !== "undefined" ? window.localStorage : null;
  } catch {
    return null;
  }
}

function decodeJwtPayload(token: string): { exp?: number } | null {
  const parts = token.split(".");
  if (parts.length !== 3) return null;
  try {
    const json = typeof atob === "function" ? atob(parts[1]) : Buffer.from(parts[1], "base64").toString("utf-8");
    return JSON.parse(json);
  } catch {
    return null;
  }
}
