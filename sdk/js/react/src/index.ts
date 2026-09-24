/** `@cratebase/react` — first-party React hooks for `@cratebase/client`.
 *
 * Two ways to use these hooks:
 *
 * 1. **Explicit client** (no provider needed): every hook accepts a
 *    `CratebaseClient` as its first argument, e.g.
 *    `useRecords(cb, "posts")`.
 * 2. **`<CratebaseProvider client={cb}>`** + the same hooks called
 *    without a client, e.g. `useRecords("posts")` — reads `cb` from
 *    context via `useCratebase()`.
 *
 * For collection-name-to-record-type inference (`useRecords("posts")`
 * knowing `"posts"`'s fields from a generated `Schema`), use
 * `createCratebaseHooks<Schema>()` instead of importing the hooks below
 * directly — see its doc comment. */

export { CratebaseProvider, useCratebase } from "./context.js";
export type { CratebaseProviderProps } from "./context.js";

export { useAuth } from "./useAuth.js";
export type { AuthState } from "./useAuth.js";

export { useRecords } from "./useRecords.js";
export type { UseRecordsOptions, UseRecordsResult } from "./useRecords.js";

export { useRecord } from "./useRecord.js";
export type { UseRecordOptions, UseRecordResult } from "./useRecord.js";

export { useInfiniteRecords } from "./useInfiniteRecords.js";
export type { UseInfiniteRecordsOptions, UseInfiniteRecordsResult } from "./useInfiniteRecords.js";

export { useMutation } from "./useMutation.js";
export type { UseMutationResult, MutationWriteOptions, OptimisticOptions } from "./useMutation.js";

export { useSubscription } from "./useSubscription.js";
export type { SubscriptionEvent } from "./useSubscription.js";

export { usePresence } from "./usePresence.js";
export type { UsePresenceOptions, UsePresenceResult } from "./usePresence.js";

export { createCratebaseHooks } from "./createHooks.js";
