/** `POST /api/queue/enqueue` — add a durable, retrying job to the
 * built-in Queue (`crates/server/src/queue.rs`). Superuser or API key,
 * the same trust tier as `_cron_jobs`/`_webhooks`: `pb.authStore` needs a
 * `_superusers` session (or an API key header) before calling this.
 * Works even while `settings.queue.enabled` is off — the job just sits
 * `pending` until an operator turns processing on, live, no restart.
 * Prefer `onQueueJob`/`$queue.enqueue` from a `pb_hooks/*.pb.js` file for
 * the handler side; this module is the client/server-script-side half.
 */

import type PocketBase from "pocketbase";

/** Options for {@link enqueue}. */
export interface EnqueueOptions {
  /** How many times the job may be attempted (including the first)
   * before it gives up for good. Server default: 5. */
  maxAttempts?: number;
  /** ISO-8601/RFC3339 timestamp; the job is not eligible to run before
   * this. Takes precedence over `delay` when both are given. */
  runAt?: string;
  /** @deprecated Use `runAt` — kept as an accepted alias since the server
   * still reads it under this name too. */
  runAfter?: string;
  /** Milliseconds from now to delay the job's first attempt. Ignored if
   * `runAt`/`runAfter` is also given. */
  delay?: number;
  /** A caller-chosen idempotency key: a second `enqueue` call with the
   * same non-empty key while an earlier job with it is still
   * pending/in-progress is a no-op — see `EnqueuedJob.deduped`. */
  dedupeKey?: string;
  /** Higher runs first among otherwise-due jobs. Server default: `0`. */
  priority?: number;
}

/** The server's `EnqueueResponse`, camelCased as PocketBase always is. */
export interface EnqueuedJob {
  id: string;
  queue: string;
  status: "pending" | string;
  runAfter: string;
  /** `true` when `options.dedupeKey` matched an already pending/
   * in-progress job — `id`/`status`/`runAfter` describe *that* job, not
   * a newly inserted one. */
  deduped: boolean;
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
 *   dedupeKey: "welcome-abc123",
 * });
 * console.log(job.id, job.status, job.deduped);
 * ```
 */
export async function enqueue(
  pb: PocketBase,
  queue: string,
  payload: unknown,
  options: EnqueueOptions = {},
): Promise<EnqueuedJob> {
  return pb.send<EnqueuedJob>("/api/queue/enqueue", {
    method: "POST",
    body: {
      queue,
      payload,
      maxAttempts: options.maxAttempts,
      runAt: options.runAt,
      runAfter: options.runAfter,
      delay: options.delay,
      dedupeKey: options.dedupeKey,
      priority: options.priority,
    },
  });
}

/** `POST /api/queue/jobs/{id}/retry` — resets a `failed` job back to
 * `pending` with a fresh `attempts` budget. */
export async function retryJob(pb: PocketBase, id: string): Promise<void> {
  await pb.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}/retry`, { method: "POST" });
}

/** `DELETE /api/queue/jobs/{id}` — removes a `_queue_jobs` row outright,
 * whatever its status. */
export async function deleteJob(pb: PocketBase, id: string): Promise<void> {
  await pb.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}`, { method: "DELETE" });
}
