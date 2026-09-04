// Single shared PocketBase client for the whole app, using the official
// `pocketbase` npm package (a real dependency here, unlike the other
// examples' esm.sh import-map trick — see README.md for why this example
// has a build step at all). Cratebase's API is byte-compatible with
// PocketBase v0.23+, so the official SDK works unchanged against it.
import PocketBase from "pocketbase";

// Defaults to the Cratebase dev server. Override with `?api=http://host:port`
// if you're running the server somewhere other than localhost:8090 (CORS is
// wide open by default).
export const BASE_URL = new URL(window.location.href).searchParams.get("api") || "http://localhost:8090";

export const pb = new PocketBase(BASE_URL);

export const CARDS_COLLECTION = "cards";
export const PRESENCE_COLLECTION = "presence";

/** Turns a PocketBase `ClientResponseError` (or anything else) into a
 * human-readable message, preferring field-level validation errors. */
export function describeError(err: unknown): string {
  const e = err as { data?: { data?: Record<string, { message?: string }>; message?: string }; message?: string };
  if (e?.data?.data) {
    const fieldErrors = Object.entries(e.data.data)
      .map(([field, info]) => `${field}: ${info?.message || "invalid"}`)
      .join("; ");
    if (fieldErrors) return fieldErrors;
  }
  if (e?.data?.message) return e.data.message;
  return e?.message || "Something went wrong.";
}
