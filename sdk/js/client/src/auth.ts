/** `client.auth` — the whole authentication surface: password/OTP/OAuth2
 * sign-in, email verification/reset/change, MFA, sessions, external
 * auths, and the admin actions (impersonate, ban). One instance is
 * bound to one auth collection (`client.auth` → `"users"` by default,
 * `client.auth.as("_superusers")` for another). */

import type { Transport } from "./transport.js";
import type { AuthStore } from "./auth-store.js";
import { MemoryAuthStore } from "./auth-store.js";
import { CratebaseError } from "./transport.js";
import type { RecordModel } from "./types.js";

export interface AuthResult<T extends RecordModel = RecordModel> {
  token: string;
  record: T;
}

export interface SignInPasswordOptions {
  identity: string;
  password: string;
  identityField?: string;
  mfaId?: string;
}

export interface SignInOtpOptions {
  otpId: string;
  code: string;
  mfaId?: string;
}

export interface SignInCodeOptions {
  provider: string;
  code: string;
  codeVerifier?: string;
  redirectUrl: string;
  createData?: Record<string, unknown>;
}

export type SocialMode = "redirect" | "manual" | "popup" | "pkce";

export interface SignInSocialOptions {
  provider: string;
  redirect?: string;
  createData?: Record<string, unknown>;
  mode?: SocialMode;
}

export interface AuthMethodsList {
  password: { enabled: boolean; identityFields: string[] };
  oauth2: { enabled: boolean; providers: Array<{ name: string; displayName: string; state: string; authURL: string; codeVerifier: string; codeChallenge: string; codeChallengeMethod: string }> };
  mfa: { enabled: boolean; duration: number };
  otp: { enabled: boolean; duration: number };
}

export interface SessionRow {
  id: string;
  kind: string;
  fingerprint: string;
  ip: string;
  userAgent: string;
  current: boolean;
  created: string;
  lastSeenAt: string | null;
  expiresAt: string;
}

/** Constructs a fresh `AuthNamespace` for another collection/store —
 * the seam `createClient` and `AuthNamespace.as()` both go through, so a
 * client instance never needs its own generic `S` parameter threaded
 * into this module. */
export type AuthFactory = (collectionName: string, store?: AuthStore) => AuthNamespace;

export class AuthNamespace {
  private readonly transport: Transport;
  private readonly collectionName: string;
  private readonly store: AuthStore;
  private readonly makeAuth: AuthFactory;

  constructor(transport: Transport, collectionName: string, store: AuthStore, makeAuth: AuthFactory) {
    this.transport = transport;
    this.collectionName = collectionName;
    this.store = store;
    this.makeAuth = makeAuth;
  }

  get token(): string {
    return this.store.token;
  }

  get record(): RecordModel | null {
    return this.store.record;
  }

  get isValid(): boolean {
    return this.store.isValid;
  }

  get isSuperuser(): boolean {
    return this.store.isSuperuser;
  }

  onChange(callback: (token: string, record: RecordModel | null) => void, fireImmediately = false): () => void {
    return this.store.onChange(callback, fireImmediately);
  }

  /** The same interface bound to another auth collection, sharing this
   * namespace's transport but never its store — the dashboard's
   * `client.auth.as("_superusers")` and this namespace's own
   * `client.auth` (bound to `"users"`) are independent sessions. */
  as(collectionName: string): AuthNamespace {
    return this.makeAuth(collectionName);
  }

  private authHeader = (): string | undefined => {
    return this.store.token ? `Bearer ${this.store.token}` : undefined;
  };

  private basePath(action: string): string {
    return `/api/collections/${encodeURIComponent(this.collectionName)}/${action}`;
  }

  /** Same URL shape as {@link basePath}, but against an explicit
   * `collection` rather than the collection this namespace's own store
   * is bound to — every `admin`/cross-collection `sessions` action below
   * targets whatever collection the record being acted on lives in,
   * which is almost never the caller's own (a superuser signed into
   * `_superusers` impersonating a `users` record, say). */
  private pathFor(collection: string | undefined, action: string): string {
    return `/api/collections/${encodeURIComponent(collection ?? this.collectionName)}/${action}`;
  }

  private applyAuthResult(result: AuthResult): AuthResult {
    this.store.save(result.token, result.record);
    return result;
  }

  async signUp(
    data: { email: string; password: string; passwordConfirm: string } & Record<string, unknown>,
    options: { autoSignIn?: boolean } = {},
  ): Promise<RecordModel> {
    const record = await this.transport.send<RecordModel>(
      `/api/collections/${encodeURIComponent(this.collectionName)}/records`,
      { method: "POST", body: data },
      this.authHeader,
    );
    if (options.autoSignIn !== false) {
      await this.signIn.password({ identity: data.email, password: data.password });
    }
    return record;
  }

  readonly signIn = {
    password: async (options: SignInPasswordOptions): Promise<AuthResult> => {
      const result = await this.transport.send<AuthResult>(
        this.basePath("auth-with-password"),
        {
          method: "POST",
          body: {
            identity: options.identity,
            password: options.password,
            identityField: options.identityField,
            mfaId: options.mfaId,
          },
        },
        this.authHeader,
      );
      return this.applyAuthResult(result);
    },
    otp: async (options: SignInOtpOptions): Promise<AuthResult> => {
      const result = await this.transport.send<AuthResult>(
        this.basePath("auth-with-otp"),
        { method: "POST", body: { otpId: options.otpId, password: options.code, mfaId: options.mfaId } },
        this.authHeader,
      );
      return this.applyAuthResult(result);
    },
    code: async (options: SignInCodeOptions): Promise<AuthResult & { meta: Record<string, unknown> }> => {
      const result = await this.transport.send<AuthResult & { meta: Record<string, unknown> }>(
        this.basePath("auth-with-oauth2"),
        {
          method: "POST",
          body: {
            provider: options.provider,
            code: options.code,
            codeVerifier: options.codeVerifier ?? "",
            redirectURL: options.redirectUrl,
            createData: options.createData ?? {},
          },
        },
        this.authHeader,
      );
      this.applyAuthResult(result);
      return result;
    },
    social: async (options: SignInSocialOptions): Promise<{ url: string } | AuthResult> => {
      const isBrowser = typeof window !== "undefined";
      const mode = options.mode ?? (isBrowser ? "redirect" : "manual");
      const redirect = options.redirect ?? (isBrowser ? window.location.href : "");
      const query = new URLSearchParams({ redirect });
      if (options.createData) {
        const encoded = btoa(JSON.stringify(options.createData)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
        query.set("createData", encoded);
      }
      const url = this.transport.buildURL(
        `${this.basePath(`oauth2/${encodeURIComponent(options.provider)}/start`)}?${query.toString()}`,
      );

      if (mode === "manual") return { url };
      if (mode === "redirect") {
        window.location.assign(url);
        return { url };
      }
      if (mode === "popup") {
        const popup = window.open(url, "cratebase-oauth2", "width=520,height=640");
        await new Promise<void>((resolve) => {
          const timer = setInterval(() => {
            if (!popup || popup.closed) {
              clearInterval(timer);
              resolve();
            }
          }, 300);
        });
        return this.refresh();
      }
      // "pkce": bearer-only fallback for deployments without cookie
      // sessions — the caller drives the popup/redirect to the
      // provider's own `authURL` itself and completes with `signIn.code`.
      throw new CratebaseError({
        status: 0,
        url,
        response: { message: '"pkce" mode requires the caller to complete signIn.code manually; call methods() to get the provider authURL.' },
      });
    },
  };

  async signOut(): Promise<void> {
    await this.transport.send<void>(this.basePath("auth-signout"), { method: "POST" }, this.authHeader);
    this.store.clear();
  }

  async refresh(): Promise<AuthResult> {
    const result = await this.transport.send<AuthResult>(
      this.basePath("auth-refresh"),
      { method: "POST" },
      this.authHeader,
    );
    return this.applyAuthResult(result);
  }

  async methods(): Promise<AuthMethodsList> {
    return this.transport.send<AuthMethodsList>(this.basePath("auth-methods"), {}, this.authHeader);
  }

  readonly otp = {
    request: async (options: { email: string }): Promise<{ otpId: string }> => {
      return this.transport.send(this.basePath("request-otp"), { method: "POST", body: options }, this.authHeader);
    },
  };

  readonly verifyEmail = {
    request: async (email: string): Promise<void> => {
      await this.transport.send(this.basePath("request-verification"), { method: "POST", body: { email } }, this.authHeader);
    },
    confirm: async (token: string): Promise<void> => {
      await this.transport.send(this.basePath("confirm-verification"), { method: "POST", body: { token } }, this.authHeader);
    },
  };

  readonly resetPassword = {
    request: async (email: string): Promise<void> => {
      await this.transport.send(this.basePath("request-password-reset"), { method: "POST", body: { email } }, this.authHeader);
    },
    confirm: async (options: { token: string; password: string; passwordConfirm: string }): Promise<void> => {
      await this.transport.send(this.basePath("confirm-password-reset"), { method: "POST", body: options }, this.authHeader);
    },
  };

  readonly changeEmail = {
    request: async (newEmail: string): Promise<void> => {
      await this.transport.send(this.basePath("request-email-change"), { method: "POST", body: { newEmail } }, this.authHeader);
    },
    confirm: async (options: { token: string; password: string }): Promise<void> => {
      await this.transport.send(this.basePath("confirm-email-change"), { method: "POST", body: options }, this.authHeader);
    },
  };

  /** Session management. `list`/`revoke*` default to *this* namespace's
   * own collection and caller — the ordinary self-service case (a
   * signed-in record managing its own sessions). Pass `collection` to
   * browse another collection's sessions as a superuser — the server
   * itself is the one enforcing that only a superuser may pass a
   * `recordId` other than the caller's own id
   * (`crate::routes::session::list_sessions`). */
  readonly sessions = {
    list: async (options: { collection?: string; recordId?: string } = {}): Promise<SessionRow[]> => {
      const res = await this.transport.send<{ items: SessionRow[] }>(
        this.pathFor(options.collection, "sessions"),
        { query: { recordId: options.recordId } },
        this.authHeader,
      );
      return res.items;
    },
    revoke: async (id: string, options: { collection?: string } = {}): Promise<void> => {
      await this.transport.send(
        this.pathFor(options.collection, `sessions/${encodeURIComponent(id)}`),
        { method: "DELETE" },
        this.authHeader,
      );
    },
    revokeOthers: async (): Promise<{ revoked: number }> => {
      return this.transport.send(this.basePath("sessions/revoke-others"), { method: "POST" }, this.authHeader);
    },
    revokeAll: async (): Promise<{ revoked: number }> => {
      const result = await this.transport.send<{ revoked: number }>(
        this.basePath("sessions/revoke-all"),
        { method: "POST" },
        this.authHeader,
      );
      this.store.clear();
      return result;
    },
  };

  readonly externalAuths = {
    list: async (recordId: string): Promise<RecordModel[]> => {
      const res = await this.transport.send<{ items: RecordModel[] }>(
        "/api/collections/_externalAuths/records",
        { query: { filter: `recordRef = "${recordId}"` } },
        this.authHeader,
      );
      return res.items;
    },
    unlink: async (recordId: string, provider: string): Promise<void> => {
      const res = await this.transport.send<{ items: RecordModel[] }>(
        "/api/collections/_externalAuths/records",
        { query: { filter: `recordRef = "${recordId}" && provider = "${provider}"` } },
        this.authHeader,
      );
      const row = res.items[0];
      if (!row) throw new CratebaseError({ status: 404, url: "", response: { message: "Not found.", status: 404 } });
      await this.transport.send(`/api/collections/_externalAuths/records/${encodeURIComponent(row.id)}`, { method: "DELETE" }, this.authHeader);
    },
  };

  /** Superuser-only actions against a record in *any* collection,
   * authenticated with *this* namespace's own token (typically a
   * `_superusers` session) — never the target collection's, which this
   * namespace usually has no session for at all. Every method takes the
   * target's collection explicitly for exactly that reason. */
  readonly admin = {
    /** Returns a *new* client auth namespace acting as `recordId`, backed
     * by a fresh `MemoryAuthStore` — the caller's own store is never
     * touched. */
    impersonate: async (
      collection: string,
      recordId: string,
      options: { duration?: number } = {},
    ): Promise<AuthNamespace> => {
      const body: Record<string, unknown> = {};
      if (options.duration) body.duration = options.duration;
      const result = await this.transport.send<AuthResult>(
        this.pathFor(collection, `impersonate/${encodeURIComponent(recordId)}`),
        { method: "POST", body },
        this.authHeader,
      );
      const impersonated = this.makeAuth(collection, new MemoryAuthStore());
      impersonated.store.save(result.token, result.record);
      return impersonated;
    },
    ban: async (
      collection: string,
      recordId: string,
      options: { reason?: string; expiresAt?: string } = {},
    ): Promise<void> => {
      await this.transport.send(
        this.pathFor(collection, `ban/${encodeURIComponent(recordId)}`),
        { method: "POST", body: options },
        this.authHeader,
      );
    },
    unban: async (collection: string, recordId: string): Promise<void> => {
      await this.transport.send(
        this.pathFor(collection, `ban/${encodeURIComponent(recordId)}`),
        { method: "DELETE" },
        this.authHeader,
      );
    },
    /** Cookie-mode only: restores the impersonator's session from the
     * `cb_session_prev` cookie `impersonate` stashed. Bearer-mode
     * "stop impersonating" is just discarding the `AuthNamespace`
     * `admin.impersonate()` returned and continuing to use the original
     * one — there is nothing server-side to call. */
    stopImpersonating: async (): Promise<RecordModel> => {
      const res = await this.transport.send<{ record: RecordModel }>(
        this.basePath("stop-impersonating"),
        { method: "POST" },
        this.authHeader,
      );
      await this.refresh();
      return res.record;
    },
  };

  /** Completes a server-driven `signIn.social()` redirect: reads
   * `cb_error`/`cb_mfa` from `search` (defaults to
   * `location.search`), throws/returns accordingly, and otherwise calls
   * `refresh()` to hydrate the store from the session cookie the
   * callback already set. */
  async completeSocial(search?: string): Promise<AuthResult | { mfaId: string }> {
    const params = new URLSearchParams(search ?? (typeof window !== "undefined" ? window.location.search : ""));
    const error = params.get("cb_error");
    if (error) {
      throw new CratebaseError({ status: 0, url: "", response: { message: error } });
    }
    const mfaId = params.get("cb_mfa");
    if (mfaId) return { mfaId };
    return this.refresh();
  }

  /** A `Set-Cookie` value for the current bearer token, for a host
   * framework managing its own cookie (server-side cookie sessions off). */
  exportCookie(name = "cb_session"): string {
    return `${name}=${this.store.token}; Path=/; HttpOnly; SameSite=Lax`;
  }
}
