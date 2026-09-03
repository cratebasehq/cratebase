import type { Cratebase } from "./client.js";
import type { RecordModel } from "./types.js";

/** A `_queue_jobs` record as returned by `POST /api/plugins/queue/enqueue`. */
export interface QueueJob extends RecordModel {
  queue: string;
  payload: unknown;
  status: "pending" | "processing" | "completed" | "failed";
  attempts: number;
  maxAttempts: number;
  availableAt: string;
  lastError?: string | null;
}

/** Client for the `queue` plugin (`crates/server/src/plugins/queue.rs`):
 * durable background job processing backed by a collection, not a
 * separate broker. Requires an authenticated caller (superuser or an
 * auth-collection record) — anonymous enqueueing is rejected server-side. */
export class QueueService {
  constructor(private readonly client: Cratebase) {}

  async enqueue(queue: string, payload?: unknown, maxAttempts?: number): Promise<QueueJob> {
    return this.client.send<QueueJob>("/api/plugins/queue/enqueue", {
      method: "POST",
      body: { queue, payload, maxAttempts },
    });
  }
}
