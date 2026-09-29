import { createClient } from "@cratebase/client";
import { createCratebaseHooks, useNotifications, useChannel, usePresence, useUpload, useMagicLinkCallback } from "@cratebase/react";
import type { Schema, SchemaCreate, SchemaUpdate } from "../cratebase-types";

const CRATEBASE_URL = process.env.NEXT_PUBLIC_CRATEBASE_URL ?? "http://localhost:8090";

// `createClient` never touches `window`/`document` at import time (its
// LocalAuthStore lazily reads localStorage on first use), so this module is
// safe to import from both server and client components. `CratebaseProvider`
// itself, and every hook below, still needs a "use client" boundary — see
// app/providers.tsx.
export const cb = createClient<Schema, SchemaCreate, SchemaUpdate>(CRATEBASE_URL);

// Collection-name-typed hooks (infer a record's shape from `Schema`).
export const { useRecords, useRecord, useInfiniteRecords, useMutation, useAuth, useCratebase, useSubscription } =
  createCratebaseHooks<Schema>();

// Not collection-typed — re-exported here too so every component can import
// everything it needs from this one module. Each resolves its client from
// `<CratebaseProvider>` context when called with no explicit client, same as
// the hooks above.
export { useNotifications, useChannel, usePresence, useUpload, useMagicLinkCallback };
