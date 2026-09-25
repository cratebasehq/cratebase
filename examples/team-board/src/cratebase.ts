// Single shared Cratebase client + typed hooks for the whole app, using
// `@cratebase/client` and `@cratebase/react`. `Schema`/`SchemaCreate`/
// `SchemaUpdate` come from `cratebase typegen` (see scripts/typegen.sh
// and README.md) — regenerate with `bun run typegen` after any
// schema.json change, or just leave the dev server running with `--dev`
// + `CB_TYPEGEN_OUT` set (scripts/dev.ts already does both), which
// regenerates it on every collection create/update/delete.
import { createClient, CratebaseError } from "@cratebase/client";
import type { RecordModel } from "@cratebase/client";
import { createCratebaseHooks } from "@cratebase/react";
import type { Schema, SchemaCreate, SchemaUpdate, UsersRecord } from "./cratebase-types.js";

/** Generated typegen records (`UsersRecord`, `CardsRecord`, ...) don't
 * declare `collectionId`/`collectionName` — `createCratebaseHooks`'s own
 * generated hooks intersect with `RecordModel` to add them (see
 * sdk/js/react/src/createHooks.ts); anything in this app calling a raw,
 * non-factory hook/helper with an explicit generic needs to do the same. */
export type WithRecordModel<T> = T & RecordModel;

// Defaults to the Cratebase dev server. Override with `?api=http://host:port`
// if you're running the server somewhere other than localhost:8090.
export const BASE_URL = new URL(window.location.href).searchParams.get("api") || "http://localhost:8090";

export const cb = createClient<Schema, SchemaCreate, SchemaUpdate>(BASE_URL);

const hooks = createCratebaseHooks<Schema>();
export const { useRecords, useRecord, useInfiniteRecords, useMutation, useCratebase } = hooks;
/** `hooks.useAuth` defaults its generic to a bare `RecordModel` — pin it
 * to the generated `UsersRecord` so `user.name`/`user.email` are typed
 * everywhere in this app instead of `unknown`. */
export const useAuth = () => hooks.useAuth<WithRecordModel<UsersRecord>>();

/** Turns a `CratebaseError` (or anything else) into a human-readable
 * message, preferring field-level validation errors. */
export function describeError(err: unknown): string {
  if (err instanceof CratebaseError) {
    const fieldErrors = Object.entries(err.response.data ?? {})
      .map(([field, info]) => `${field}: ${(info as { message?: string } | null)?.message || "invalid"}`)
      .join("; ");
    if (fieldErrors) return fieldErrors;
    if (err.response.message) return err.response.message;
  }
  return err instanceof Error ? err.message : "Something went wrong.";
}
