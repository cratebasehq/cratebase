import PocketBase, { ClientResponseError } from "pocketbase";

/** Single shared client for the whole dashboard. `authStore` persists the
 * superuser session to `localStorage` under this key so a page reload
 * doesn't require logging back in.
 *
 * The default has to be `"/"`, not `""`: a base URL that doesn't start with
 * a slash is resolved by the SDK against `window.location.pathname`, so on
 * `/collections/posts` every call would go to
 * `/collections/posts/api/...`, get the SPA's own `index.html` back, fail
 * to parse as JSON and resolve to `{}` — a silent, shapeless success. */
export const cb = new PocketBase(import.meta.env.VITE_API_URL ?? "/");

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
}

export function isLoggedIn(): boolean {
  return cb.authStore.isValid && cb.authStore.record !== null;
}

/** The signed-in superuser, or `null`. Reads straight off the auth store so
 * it stays correct after a login, a logout, or a page reload. */
export function currentSuperuser(): SuperuserRecord | null {
  const record = cb.authStore.record;
  if (!record) return null;
  return {
    id: record.id,
    email: typeof record["email"] === "string" ? record["email"] : "",
    collectionName: typeof record["collectionName"] === "string" ? record["collectionName"] : undefined,
    created: typeof record["created"] === "string" ? record["created"] : undefined,
    verified: record["verified"] === true,
    avatar: typeof record["avatar"] === "string" ? record["avatar"] : undefined,
  };
}

/**
 * Superuser login. PocketBase v0.23+ dropped `/api/admins/*` in favour of
 * the `_superusers` auth collection, and the field is `identity` (an email
 * *or* username) rather than `email`.
 */
export async function authWithPassword(identity: string, password: string) {
  return cb.collection("_superusers").authWithPassword<SuperuserRecord>(identity, password);
}

export function signOut(): void {
  cb.authStore.clear();
}

/** `GET /api/health` — unauthenticated, so it doubles as a reachability
 * probe for the login screen: it answers "is `cratebase serve` even
 * running?", which is the failure the old login reported as
 * "Invalid email or password." */
export async function checkHealth(): Promise<{ message: string }> {
  return cb.send<{ message: string }>("/api/health", { method: "GET" });
}

/* ------------------------------------------------------------------------ *
 * Error handling
 * ------------------------------------------------------------------------ */

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

/** The SDK synthesises `request to <url> failed with status <n>` when a
 * response carries no JSON body. That is a restatement of the status line,
 * not a message from the server, so it must not be shown as one. */
function serverSentence(error: ClientResponseError): string {
  return /^request to .+ failed with status \d+$/.test(error.message) ? "" : error.message;
}

function fieldErrors(error: ClientResponseError): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [name, entry] of Object.entries(error.data ?? {})) {
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
  if (error instanceof ClientResponseError) {
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
  if (cb.baseURL) return cb.baseURL;
  return typeof window === "undefined" ? "the API" : window.location.origin;
}

/** True for the one failure the whole app must react to identically: the
 * session is gone. 403 is deliberately excluded — that is a rule denial on a
 * request the session was allowed to make, and signing the user out for it
 * would hide the real problem. */
export function isSessionExpired(error: unknown): boolean {
  return error instanceof ClientResponseError && error.status === 401;
}
