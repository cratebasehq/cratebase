/** `POST /api/plugins/queue/enqueue` — add a durable, retrying job to
 * the toggle-gated built-in Queue plugin (`crates/server/src/queue.rs`,
 * `settings.queue.enabled`). Superuser-only, the same trust tier as
 * `_cron_jobs`/`_webhooks`: `pb.authStore` needs a `_superusers` session
 * before calling this.
 */

import type PocketBase from "pocketbase";

/** Options for {@link enqueue}. */
export interface EnqueueOptions {
  /** How many times the job may be attempted (including the first)
   * before it gives up for good. Server default: 5. */
  maxAttempts?: number;
  /** ISO-8601/RFC3339 timestamp; the job is not eligible to run before
   * this. Server default: now. */
  runAfter?: string;
}

/** The server's `EnqueueResponse`, camelCased as PocketBase always is. */
export interface EnqueuedJob {
  id: string;
  queue: string;
  status: "pending";
  runAfter: string;
}

/** Enqueue a job on `queue` with `payload`.
 *
 * ```ts
 * import PocketBase from "pocketbase";
 * import { enqueue } from "@cratebase/extras";
 *
 * const pb = new PocketBase("http://127.0.0.1:8090");
 * await pb.collection("_superusers").authWithPassword(email, password);
 *
 * const job = await enqueue(pb, "send-welcome-email", { userId: "abc123" }, {
 *   maxAttempts: 3,
 * });
 * console.log(job.id, job.status);
 * ```
 */
export async function enqueue(
  pb: PocketBase,
  queue: string,
  payload: unknown,
  options: EnqueueOptions = {},
): Promise<EnqueuedJob> {
  return pb.send<EnqueuedJob>("/api/plugins/queue/enqueue", {
    method: "POST",
    body: { queue, payload, maxAttempts: options.maxAttempts, runAfter: options.runAfter },
  });
}
