import { createClient } from "@cratebase/client";
import { createCratebaseHooks, useNotifications, useChannel, usePresence, useUpload, useMagicLinkCallback } from "@cratebase/react";
import type { Schema, SchemaCreate, SchemaUpdate } from "../../cratebase-types";

const CRATEBASE_URL = import.meta.env.VITE_CRATEBASE_URL ?? "http://localhost:8090";

export const cb = createClient<Schema, SchemaCreate, SchemaUpdate>(CRATEBASE_URL);

// Collection-name-typed hooks (infer a record's shape from `Schema`).
export const { useRecords, useRecord, useInfiniteRecords, useMutation, useAuth, useCratebase, useSubscription } =
  createCratebaseHooks<Schema>();

// Not collection-typed — re-exported here too so every component can import
// everything it needs from this one module. Each resolves its client from
// `<CratebaseProvider>` context when called with no explicit client, same as
// the hooks above.
export { useNotifications, useChannel, usePresence, useUpload, useMagicLinkCallback };

// `search` matches whole words by default (SQLite FTS5 tokens): typing "sear"
// finds nothing until the word is complete. A search-as-you-type box wants
// prefix matching, which FTS5 spells `term*`. This keeps letters/digits only
// (FTS5 punctuation can't reach the query parser), splits on everything else
// the way FTS5's tokenizer does, and appends `*` to every word. On Postgres
// the `*` is ignored by `websearch_to_tsquery`, so search falls back to
// whole-word matching there.
export function toPrefixSearch(input: string): string | undefined {
  const words = input.split(/[^\p{L}\p{N}_]+/u).filter(Boolean);
  return words.length ? words.map((w) => `${w}*`).join(" ") : undefined;
}
