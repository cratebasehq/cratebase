/**
 * Server-side glue for TanStack Start apps on Cratebase. Server-only: it
 * reads the incoming request and writes `Set-Cookie`, so import it from
 * `*.server.ts` modules or inside server-function handlers.
 */
import { createMiddleware } from "@tanstack/react-start";
import { getRequest, setResponseHeader } from "@tanstack/react-start/server";
import { createClient, type AnySchema, type CratebaseClient, type RecordModel } from "@cratebase/client";

export interface CratebaseStartOptions {
  /** Cratebase base URL, e.g. `http://127.0.0.1:8090`. */
  url: string;
  /** A `cb_…` API key, used by `adminClient()`. */
  apiKey?: string;
  /** Session cookie name. Default `"cb_session"`. */
  cookieName?: string;
  /** Auth collection backing the session. Default `"users"`. */
  authCollection?: string;
  /** Session cookie lifetime. Default 30 days. */
  maxAgeSeconds?: number;
  /** How long `getUser()` trusts a verified token. Default 240 000 ms. */
  sessionCacheTtlMs?: number;
}

const CACHE_CAP = 1000;

function requestIsSecure(): boolean {
  const req = getRequest();
  const forwarded = req.headers.get("x-forwarded-proto");
  if (forwarded) return forwarded.split(",")[0]!.trim() === "https";
  try {
    return new URL(req.url).protocol === "https:";
  } catch {
    return false;
  }
}

function readCookie(header: string, name: string): string | null {
  for (const part of header.split(";")) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    if (part.slice(0, eq).trim() !== name) continue;
    const raw = part.slice(eq + 1).trim();
    try {
      return decodeURIComponent(raw);
    } catch {
      return raw;
    }
  }
  return null;
}

/** JWT `exp` in ms, or `undefined` when the token isn't a JWT / has none. */
function jwtExpiryMs(token: string): number | undefined {
  const parts = token.split(".");
  if (parts.length !== 3) return undefined;
  try {
    const payload = JSON.parse(atob(parts[1]!.replace(/-/g, "+").replace(/_/g, "/"))) as { exp?: number };
    return typeof payload.exp === "number" ? payload.exp * 1000 : undefined;
  } catch {
    return undefined;
  }
}

export function createCratebaseStart<S extends AnySchema = Record<string, RecordModel>>(opts: CratebaseStartOptions) {
  const cookieName = opts.cookieName ?? "cb_session";
  const authCollection = opts.authCollection ?? "users";
  const maxAgeSeconds = opts.maxAgeSeconds ?? 2_592_000;
  const cacheTtlMs = opts.sessionCacheTtlMs ?? 240_000;

  const cache = new Map<string, { record: RecordModel; expires: number }>();
  const inflight = new Map<string, Promise<RecordModel | null>>();

  function sessionCookie(token: string, cb: CratebaseClient<S>): string {
    const secure = requestIsSecure();
    return token
      ? cb.auth.exportCookie(cookieName, { secure, maxAge: maxAgeSeconds })
      : `${cookieName}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0${secure ? "; Secure" : ""}`;
  }

  function client(): CratebaseClient<S> {
    const cb = createClient<S>(opts.url, {
      cookie: getRequest().headers.get("cookie") ?? "",
      authCollection,
    });
    cb.auth.onChange((token) => setResponseHeader("Set-Cookie", sessionCookie(token, cb)));
    return cb;
  }

  function adminClient(): CratebaseClient<S> {
    if (!opts.apiKey) {
      throw new Error("createCratebaseStart: apiKey is required for adminClient()");
    }
    return createClient<S>(opts.url, { token: opts.apiKey });
  }

  function requestToken(): string | null {
    return readCookie(getRequest().headers.get("cookie") ?? "", cookieName) || null;
  }

  async function getUser(options: { fresh?: boolean } = {}): Promise<RecordModel | null> {
    const token = requestToken();
    if (!token) return null;

    const exp = jwtExpiryMs(token);
    if (exp !== undefined && exp <= Date.now()) return null;

    if (!options.fresh) {
      const hit = cache.get(token);
      if (hit && hit.expires > Date.now()) return hit.record;
      if (hit) cache.delete(token);
    }

    const pending = inflight.get(token);
    if (pending) return pending;

    const run = (async (): Promise<RecordModel | null> => {
      try {
        const { record } = await client().auth.refresh();
        if (cache.size >= CACHE_CAP && !cache.has(token)) {
          const oldest = cache.keys().next().value;
          if (oldest !== undefined) cache.delete(oldest);
        }
        cache.set(token, { record, expires: Date.now() + cacheTtlMs });
        return record;
      } catch {
        cache.delete(token);
        return null;
      } finally {
        inflight.delete(token);
      }
    })();
    inflight.set(token, run);
    return run;
  }

  function setSession(token: string): void {
    const secure = requestIsSecure();
    const parts = [`${cookieName}=${token}`, "Path=/", "HttpOnly", "SameSite=Lax"];
    if (secure) parts.push("Secure");
    parts.push(`Max-Age=${maxAgeSeconds}`);
    setResponseHeader("Set-Cookie", parts.join("; "));
  }

  function clearSession(): void {
    const token = requestToken();
    if (token) {
      cache.delete(token);
      inflight.delete(token);
    }
    setResponseHeader(
      "Set-Cookie",
      `${cookieName}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0${requestIsSecure() ? "; Secure" : ""}`,
    );
  }

  const authMiddleware = createMiddleware().server(async ({ next }) => {
    const user = await getUser();
    if (!user) throw new Response("Unauthorized", { status: 401 });
    return next({ context: { user } });
  });

  const freshAuthMiddleware = createMiddleware().server(async ({ next }) => {
    const user = await getUser({ fresh: true });
    if (!user) throw new Response("Unauthorized", { status: 401 });
    return next({ context: { user } });
  });

  const optionalAuthMiddleware = createMiddleware().server(async ({ next }) => {
    const user = await getUser();
    return next({ context: { user: user ?? null } });
  });

  return { client, adminClient, getUser, setSession, clearSession, authMiddleware, freshAuthMiddleware, optionalAuthMiddleware };
}
