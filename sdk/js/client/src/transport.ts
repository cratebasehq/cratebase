/** The HTTP transport every module sends through: URL building, request
 * encoding, and the error shape every failure surfaces as.
 *
 * No auto-cancellation. PocketBase's SDK auto-cancels same-key in-flight
 * requests (why call sites there sprinkle `requestKey: null`); this
 * client has no such behavior, so there is nothing to opt out of —
 * callers pass an `AbortSignal` when they want cancellation. */

/** Every failure this client throws — a non-2xx response, a malformed
 * response body, or an aborted request. */
export class CratebaseError extends Error {
  readonly status: number;
  readonly url: string;
  readonly response: {
    status: number;
    message: string;
    data: Record<string, { code: string; message: string }>;
  };
  readonly isAbort: boolean;
  /** Set only for the server's bare `401 {"mfaId": "..."}` shape
   * (`crates/server/src/routes/auth.rs`'s `mfa_pending_response`), which
   * is deliberately not wrapped in the usual `{status, message, data}`
   * envelope so this is the one place a 401 is not necessarily "this
   * request failed" but "supply a second factor". */
  readonly mfaId?: string;

  constructor(init: {
    status: number;
    url: string;
    response?: unknown;
    isAbort?: boolean;
    mfaId?: string;
  }) {
    const response =
      init.response && typeof init.response === "object"
        ? (init.response as Record<string, unknown>)
        : {};
    const message =
      typeof response.message === "string" ? response.message : `Request failed with status ${init.status}.`;
    super(message);
    this.name = "CratebaseError";
    this.status = init.status;
    this.url = init.url;
    this.isAbort = init.isAbort ?? false;
    this.mfaId = init.mfaId;
    this.response = {
      status: typeof response.status === "number" ? response.status : init.status,
      message,
      data:
        response.data && typeof response.data === "object"
          ? (response.data as Record<string, { code: string; message: string }>)
          : {},
    };
  }
}

export type QueryValue = string | number | boolean | null | undefined;

export interface SendOptions {
  method?: string;
  query?: Record<string, QueryValue>;
  body?: unknown;
  headers?: Record<string, string>;
  signal?: AbortSignal;
  /** Override the client's own `fetch` for this one call. */
  fetch?: typeof fetch;
}

export interface TransportOptions {
  fetch?: typeof fetch;
  /** Sent with every request; a caller managing its own cookie forwarding
   * (SSR) sets this to the incoming request's `Cookie` header. */
  cookie?: string;
  headers?: Record<string, string>;
  credentials?: RequestCredentials;
  /** `Accept-Language`, PocketBase's `lang` header equivalent. */
  lang?: string;
}

/** Joins `baseUrl` and `path` without producing `//api/...` when
 * `baseUrl` is `"/"` — a naive `${baseUrl}${path}` template does exactly
 * that (a protocol-relative URL resolved against a host literally named
 * `api`), the bug documented at
 * `web/admin/src/components/settings/file-manager-page.tsx:68-77`. */
export function joinURL(baseUrl: string, path: string): string {
  const base = baseUrl.endsWith("/") ? baseUrl.slice(0, -1) : baseUrl;
  const rest = path.startsWith("/") ? path : `/${path}`;
  return `${base}${rest}` || "/";
}

function appendQuery(url: string, query: Record<string, QueryValue> | undefined): string {
  if (!query) return url;
  const parts: string[] = [];
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null) continue;
    parts.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`);
  }
  if (parts.length === 0) return url;
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}${parts.join("&")}`;
}

export class Transport {
  readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private readonly cookie?: string;
  private readonly extraHeaders: Record<string, string>;
  readonly credentials: RequestCredentials;
  private readonly lang?: string;

  constructor(baseUrl: string, options: TransportOptions = {}) {
    this.baseUrl = baseUrl;
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.cookie = options.cookie;
    this.extraHeaders = options.headers ?? {};
    this.credentials = options.credentials ?? "same-origin";
    this.lang = options.lang;
  }

  buildURL(path: string): string {
    return joinURL(this.baseUrl, path);
  }

  /** `authHeader` is read fresh on every call (not captured at
   * construction) so a client's own `Transport` always sends whatever
   * the attached `AuthStore` currently holds. */
  async send<T>(path: string, options: SendOptions = {}, authHeader?: () => string | undefined): Promise<T> {
    const url = appendQuery(this.buildURL(path), options.query);
    const fetchImpl = options.fetch ?? this.fetchImpl;
    const headers: Record<string, string> = { ...this.extraHeaders, ...options.headers };
    if (this.lang && !headers["Accept-Language"]) headers["Accept-Language"] = this.lang;
    if (this.cookie && !headers["Cookie"]) headers["Cookie"] = this.cookie;
    const token = authHeader?.();
    if (token && !headers["Authorization"]) headers["Authorization"] = token;

    let body: BodyInit | undefined;
    if (options.body instanceof FormData) {
      body = options.body;
    } else if (options.body !== undefined) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(options.body);
    }

    let res: Response;
    try {
      res = await fetchImpl(url, {
        method: options.method ?? "GET",
        headers,
        body,
        signal: options.signal,
        credentials: this.credentials,
      });
    } catch (err) {
      if (err instanceof DOMException && err.name === "AbortError") {
        throw new CratebaseError({ status: 0, url, isAbort: true });
      }
      throw new CratebaseError({ status: 0, url, response: { message: String(err) } });
    }

    if (res.status === 204) {
      return undefined as T;
    }

    const text = await res.text();
    let parsed: unknown = undefined;
    if (text.length > 0) {
      try {
        parsed = JSON.parse(text);
      } catch {
        // Non-JSON body (e.g. a redirect landing page) — leave undefined.
      }
    }

    if (!res.ok) {
      if (
        res.status === 401 &&
        parsed &&
        typeof parsed === "object" &&
        "mfaId" in (parsed as Record<string, unknown>) &&
        !("status" in (parsed as Record<string, unknown>))
      ) {
        throw new CratebaseError({
          status: res.status,
          url,
          mfaId: String((parsed as Record<string, unknown>).mfaId),
          response: { message: "MFA required.", status: res.status },
        });
      }
      throw new CratebaseError({ status: res.status, url, response: parsed });
    }

    return parsed as T;
  }
}
