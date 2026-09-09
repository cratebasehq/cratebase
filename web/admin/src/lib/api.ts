import { createClient, CratebaseError } from "@cratebase/client";

/** Single shared client for the whole dashboard. The default
 * `LocalAuthStore` persists the superuser session to `localStorage` under
 * `cratebase_auth` so a page reload doesn't sign the user back out.
 *
 * `baseUrl` is `"/"` in production (the dashboard is served from the same
 * origin as the API) and the dev server's Vite proxy in development —
 * `cb.buildURL`/`cb.send` handle joining that against `/api/...` without
 * producing `//api/...`.
 *
 * `authCollection: "_superusers"` matters beyond `client.auth` itself:
 * every non-collection-scoped call this dashboard makes — `cb.admin.*`
 * (settings, backups, logs, schema, storage, api keys, crons, push,
 * sql), `cb.send()`, `cb.files`, `cb.realtime` — authenticates as
 * *whichever* collection the client was constructed with, defaulting to
 * `"users"`. Every dashboard page signs in as a superuser, never as an
 * ordinary `users` record, so leaving the default would silently send
 * every one of those requests with no bearer token at all. */
export const cb = createClient(import.meta.env.VITE_API_URL ?? "/", { authCollection: "_superusers" });

/** `client.auth`, already bound to `_superusers` by the `authCollection`
 * option above — named for what it is at every dashboard call site. */
export const superuserAuth = cb.auth;

/** The superuser record returned by `auth-with-password`. Superusers are
 * ordinary auth records in the `_superusers` collection (PocketBase v0.23+),
 * so this is a record shape, not a bespoke "admin" model. */
export interface SuperuserRecord {
  id: string;
  email: string;
  collectionId?: string;
  collectionName?: string;
  created?: string;
  updated?: string;
  verified?: boolean;
  avatar?: string;
  role?: string;
}

export function isLoggedIn(): boolean {
  return superuserAuth.isValid && superuserAuth.record !== null;
}

/** The signed-in superuser, or `null`. Reads straight off the auth store so
 * it stays correct after a login, a logout, or a page reload. */
export function currentSuperuser(): SuperuserRecord | null {
  const record = superuserAuth.record;
  if (!record) return null;
  return {
    id: record.id,
    email: typeof record["email"] === "string" ? record["email"] : "",
    collectionId: typeof record["collectionId"] === "string" ? record["collectionId"] : undefined,
    collectionName: typeof record["collectionName"] === "string" ? record["collectionName"] : undefined,
    created: typeof record["created"] === "string" ? record["created"] : undefined,
    updated: typeof record["updated"] === "string" ? record["updated"] : undefined,
    verified: record["verified"] === true,
    avatar: typeof record["avatar"] === "string" ? record["avatar"] : undefined,
    role: typeof record["role"] === "string" ? record["role"] : undefined,
  };
}

/** `GET /api/utils/avatar/{seed}` — a deterministic, hashed placeholder
 * avatar (`crates/server/src/routes/utils.rs`) for anywhere a record has
 * no uploaded avatar of its own. `Cache-Control: max-age=31536000,
 * immutable` on the response means the same seed is free on subsequent
 * reloads without the server storing anything. */
export function avatarUrl(seed: string, size = 64): string {
  return cb.buildURL(`/api/utils/avatar/${encodeURIComponent(seed)}?size=${size}`);
}

/**
 * Superuser login. PocketBase v0.23+ dropped `/api/admins/*` in favour of
 * the `_superusers` auth collection, and the field is `identity` (an email
 * *or* username) rather than `email`.
 */
export async function authWithPassword(identity: string, password: string) {
  return superuserAuth.signIn.password({ identity, password });
}

export function signOut(): void {
  superuserAuth.signOut().catch(() => {
    // Best-effort server-side revocation; the local store is cleared
    // either way so the UI signs out immediately.
  });
}

/** `GET /api/health` — unauthenticated, so it doubles as a reachability
 * probe for the login screen: it answers "is `cratebase serve` even
 * running?", which is the failure the old login reported as
 * "Invalid email or password." */
export async function checkHealth(): Promise<{ message: string }> {
  return cb.send<{ message: string }>("/api/health");
}

/** `GET /api/health`, authenticated — a superuser gets an extra `data`
 * envelope carrying `canBackup`: whether this server's storage backend
 * (SQLite, not Postgres) and `backups_storage` are both configured for
 * backups. Used by the backups page to hide actions that would just 403. */
export async function checkBackupCapability(): Promise<boolean> {
  const res = await cb.send<{ data?: { canBackup?: boolean } }>("/api/health");
  return res.data?.canBackup === true;
}

/** `GET /api/setup/status` — unauthenticated. Tells the login screen
 * whether to render the ordinary login form or the first-run "create your
 * first superuser" form. */
export async function checkSetupStatus(): Promise<{ needsSetup: boolean }> {
  return cb.send<{ needsSetup: boolean }>("/api/setup/status");
}

/** `POST /api/setup` — unauthenticated, and only succeeds once: the server
 * rejects it with 403 as soon as any superuser exists. Does not sign in —
 * the caller follows up with {@link authWithPassword} using the same
 * credentials, exactly like a normal login. */
export async function createFirstSuperuser(
  email: string,
  password: string,
  passwordConfirm: string,
): Promise<void> {
  await cb.send<void>("/api/setup", {
    method: "POST",
    body: { email, password, passwordConfirm },
  });
}

/** A failure translated into something a person can act on. `fields` carries
 * the per-field entries of the `{status, message, data}` envelope so a form
 * can mark the offending input instead of firing a generic toast. */
export interface ApiFailure {
  /** HTTP status, or 0 when the request never reached a server. */
  status: number;
  /** One line naming what happened. */
  title: string;
  /** One line naming what to do about it. Empty when there is nothing useful to add. */
  detail: string;
  /** The server's own `message`, verbatim, for anywhere that shows the raw
   * envelope. Empty when the request never reached a server. */
  serverMessage: string;
  /** Field name → message, from the envelope's `data`. */
  fields: Record<string, string>;
  /** True when the browser could not reach the server at all. */
  offline: boolean;
}

/** The client synthesises a generic message when a response carries no
 * JSON body. That is a restatement of the status line, not a message from
 * the server, so it must not be shown as one. */
function serverSentence(error: CratebaseError): string {
  return /^Request failed with status \d+\.$/.test(error.message) ? "" : error.message;
}

function fieldErrors(error: CratebaseError): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [name, entry] of Object.entries(error.response.data ?? {})) {
    if (entry && typeof entry === "object" && typeof entry.message === "string") {
      out[name] = entry.message;
    }
  }
  return out;
}

/**
 * Turn any thrown value into an `ApiFailure`.
 *
 * The old dashboard rendered "Invalid email or password." for every failure
 * including timeouts and 500s, which sent people hunting for a typo in a
 * password that was never the problem. Each status now says a different,
 * true thing.
 */
export function describeFailure(error: unknown, context: "auth" | "generic" = "generic"): ApiFailure {
  if (error instanceof CratebaseError) {
    const fields = fieldErrors(error);
    const message = serverSentence(error);
    const base = { status: error.status, fields, offline: false, serverMessage: message };

    if (error.status === 0) {
      return {
        ...base,
        offline: true,
        title: "Can't reach the server",
        detail: `No response from ${serverOrigin()}. Check that Cratebase is running.`,
      };
    }
    if (error.status === 400) {
      return {
        ...base,
        title: context === "auth" ? "Those credentials didn't work" : "The server rejected that request",
        detail:
          Object.keys(fields).length > 0
            ? ""
            : message || "Check the highlighted fields and try again.",
      };
    }
    if (error.status === 401 || error.status === 403) {
      return {
        ...base,
        title: context === "auth" ? "Those credentials didn't work" : "You're not allowed to do that",
        detail:
          context === "auth"
            ? "No superuser account matches that email and password."
            : "Your session may have expired. Sign in again.",
      };
    }
    if (error.status === 404) {
      return {
        ...base,
        title: "Not found",
        detail:
          context === "auth"
            ? "This server has no superuser auth endpoint. It may be running an older Cratebase."
            : "That record or collection no longer exists.",
      };
    }
    if (error.status === 429) {
      return { ...base, title: "Too many attempts", detail: "Wait a moment, then try again." };
    }
    // 502/503/504 come from whatever sits in front of Cratebase — a reverse
    // proxy, or the Vite dev proxy — and mean the backend itself is not
    // answering, which is a different fix from "the server threw".
    if (error.status === 502 || error.status === 503 || error.status === 504) {
      return {
        ...base,
        title: "Cratebase isn't answering",
        detail: `The proxy in front of it returned ${error.status}. Check that the server is running at ${serverOrigin()}.`,
      };
    }
    if (error.status >= 500) {
      return {
        ...base,
        title: `The server errored (${error.status})`,
        detail: message || "Check the Cratebase server logs for the failing request.",
      };
    }
    return { ...base, title: message || "Request failed", detail: "" };
  }

  // `fetch` throws a TypeError for DNS failures, refused connections, CORS
  // and offline — none of which are the user's password.
  return {
    status: 0,
    fields: {},
    offline: true,
    serverMessage: "",
    title: "Can't reach the server",
    detail: `No response from ${serverOrigin()}. Check that Cratebase is running.`,
  };
}

function serverOrigin(): string {
  const base = cb.buildURL("/");
  try {
    return new URL(base, window.location.href).origin;
  } catch {
    return base;
  }
}

/** True for the one failure the whole app must react to identically: the
 * session is gone. 403 is deliberately excluded — that is a rule denial on a
 * request the session was allowed to make, and signing the user out for it
 * would hide the real problem. */
export function isSessionExpired(error: unknown): boolean {
  return error instanceof CratebaseError && error.status === 401;
}

/** The server writes PocketBase's datetime form (a space, not a `T`,
 * e.g. `2026-01-31 12:00:00.000Z`), which `new Date()` does not parse in
 * every browser — Safari returns Invalid Date for it. Every call site that
 * parses a server-supplied timestamp (`created`, `updated`, `modified`,
 * …) must go through this, not a bare `new Date(...)`. */
export function parseServerDate(value: string): Date {
  return new Date(value.replace(" ", "T"));
}
