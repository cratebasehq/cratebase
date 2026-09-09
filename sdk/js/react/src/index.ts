/** `@cratebase/react` — first-party React hooks for `@cratebase/client`.
 * Every hook here takes an existing `CratebaseClient` instance as its
 * first argument; none of them construct their own client, matching
 * `@cratebase/extras`' "bolt onto an existing client" convention. */

export { useAuth } from "./useAuth.js";
export type { AuthState } from "./useAuth.js";
export { useRecords } from "./useRecords.js";
export type { UseRecordsOptions, UseRecordsResult } from "./useRecords.js";
export { useRecord } from "./useRecord.js";
export type { UseRecordOptions, UseRecordResult } from "./useRecord.js";
export { useSubscription } from "./useSubscription.js";
export type { SubscriptionEvent } from "./useSubscription.js";
