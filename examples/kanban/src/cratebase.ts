// Single shared Cratebase client for the whole app, using the first-party
// `@cratebase/client` SDK (consumed from source via the Vite alias in
// `vite.config.ts` — this package isn't published to npm yet; see
// README.md). Typed CRUD, realtime, and auth all come from the same
// client instance, no separate PocketBase-family SDK involved.
import { createClient, CratebaseError } from "@cratebase/client";

// Defaults to the Cratebase dev server. Override with `?api=http://host:port`
// if you're running the server somewhere other than localhost:8090 (CORS is
// wide open by default).
export const BASE_URL = new URL(window.location.href).searchParams.get("api") || "http://localhost:8090";

export const cb = createClient(BASE_URL);

export const CARDS_COLLECTION = "cards";
export const PRESENCE_COLLECTION = "presence";

/** Turns a `CratebaseError` (or anything else) into a human-readable
 * message, preferring field-level validation errors. */
export function describeError(err: unknown): string {
  if (err instanceof CratebaseError) {
    const fieldErrors = Object.entries(err.response.data)
      .map(([field, info]) => `${field}: ${info?.message || "invalid"}`)
      .join("; ");
    if (fieldErrors) return fieldErrors;
    if (err.response.message) return err.response.message;
  }
  return err instanceof Error ? err.message : "Something went wrong.";
}
