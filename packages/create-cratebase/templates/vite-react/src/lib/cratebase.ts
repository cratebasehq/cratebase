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
