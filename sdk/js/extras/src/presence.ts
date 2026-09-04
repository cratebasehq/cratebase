/** Presence: "who's online right now" over an ordinary collection —
 * needs no new server capability, just a pattern layered on top of two
 * things Cratebase already ships: the record CRUD API and
 * `GET /api/realtime` (`crates/server/src/realtime.rs`).
 *
 * A presence collection is a plain app-defined collection (a `presence`
 * base collection with `userRef`, `lastSeenAt`, `status` fields is a
 * reasonable starting shape; see the README's example) whose rows you
 * keep alive with a periodic heartbeat and observe with an ordinary
 * realtime subscription. {@link trackPresence} does both sides of that:
 *
 * - **publish**: upserts your own row every `heartbeatMs`, so its
 *   `lastSeenAt`-style field never goes stale while your tab is open;
 * - **observe**: subscribes to every change on the collection and keeps
 *   a local `online` set of row ids whose heartbeat is still fresh,
 *   sweeping out any row that has gone quiet for `staleMs` — the only
 *   way to notice a peer that disappeared without a clean "going
 *   offline" signal (closed laptop, dead network, crashed tab).
 *
 * This is the same shape Supabase's and Firebase's presence helpers use
 * (heartbeat + realtime + client-side staleness), adapted to Cratebase's
 * existing primitives rather than a bespoke presence protocol — there is
 * nothing here the server needs to know about.
 */

import type PocketBase from "pocketbase";
import type { RecordModel, RecordSubscription } from "pocketbase";

/** Options for {@link trackPresence}. */
export interface PresenceOptions {
  /** How often to refresh the local row's heartbeat field, in
   * milliseconds. Default 20 000 (20s) — frequent enough that a peer
   * leaving reads as "offline" within a few heartbeats, infrequent
   * enough not to spam the collection's `updateRule`. */
  heartbeatMs?: number;
  /** How long a row may go without a heartbeat before it is swept out
   * of {@link Presence.online}. Default `3 * heartbeatMs`, matching the
   * usual "missed a couple of beats" tolerance for network jitter
   * without waiting so long that a closed tab still reads as online. */
  staleMs?: number;
  /** The field on each row that carries the last-heartbeat timestamp.
   * Default `"lastSeenAt"`. */
  lastSeenField?: string;
  /** Extra fields merged into the row on every heartbeat (and the
   * initial create), e.g. `{ status: "online" }` or app-specific
   * metadata such as a current page or activity. */
  data?: Record<string, unknown>;
}

/** The live handle returned by {@link trackPresence}. */
export interface Presence {
  /** ids of the presence rows currently considered online — this
   * row's own id is included once its first heartbeat has landed. */
  readonly online: Set<string>;
  /** Called with the current {@link online} set whenever it changes
   * (a peer's row is created/updated/goes stale/is deleted), including
   * once immediately with the set as of the call. Returns a function
   * that removes just this listener. */
  subscribe(callback: (online: Set<string>) => void): () => void;
  /** Stop heartbeating and tear down the realtime subscription and the
   * staleness sweep. Does not delete this row — its own heartbeat
   * simply stops, so peers see it drop out of their `online` set after
   * `staleMs`, the same as any other disconnect. */
  stop(): Promise<void>;
}

/** Track presence on `collectionIdOrName`: keep `record` alive with a
 * heartbeat and observe every other row's, exposing a live `online` set
 * of row ids.
 *
 * `record` is either `{ id, ...fields }` for a row you already created
 * (its heartbeat field is refreshed in place) or fields with no `id`,
 * in which case the row is created on the first call and reused for
 * every heartbeat after that — either way this is the "upsert a
 * heartbeat record" half of the pattern.
 *
 * ```ts
 * import PocketBase from "pocketbase";
 * import { trackPresence } from "@cratebase/extras";
 *
 * const pb = new PocketBase("http://127.0.0.1:8090");
 * await pb.collection("users").authWithPassword(email, password);
 *
 * const presence = await trackPresence(pb, "presence", {
 *   userRef: pb.authStore.record!.id,
 *   status: "online",
 * });
 *
 * presence.subscribe((online) => {
 *   console.log(`${online.size} peers online`);
 * });
 *
 * // later, e.g. on unmount:
 * await presence.stop();
 * ```
 */
export async function trackPresence<T extends RecordModel = RecordModel>(
  pb: PocketBase,
  collectionIdOrName: string,
  record: ({ id: string } | Record<string, unknown>) & Record<string, unknown>,
  options: PresenceOptions = {},
): Promise<Presence> {
  const heartbeatMs = options.heartbeatMs ?? 20_000;
  const staleMs = options.staleMs ?? heartbeatMs * 3;
  const lastSeenField = options.lastSeenField ?? "lastSeenAt";
  const extraData = options.data ?? {};
  const collection = pb.collection<T>(collectionIdOrName);

  const beat = () => ({ ...extraData, [lastSeenField]: new Date().toISOString() });

  let ownId = typeof record.id === "string" ? record.id : undefined;
  if (ownId) {
    await collection.update(ownId, { ...record, ...beat() });
  } else {
    const created = await collection.create({ ...record, ...beat() });
    ownId = created.id;
  }

  // Rows keyed by id, so a stale sweep can recompute `online` from
  // scratch without re-fetching — every row this client has ever seen
  // a create/update for, including its own.
  const rows = new Map<string, T>();
  const seed = await collection.getFullList<T>();
  for (const row of seed) rows.set(row.id, row);

  const listeners = new Set<(online: Set<string>) => void>();
  let online = new Set<string>();

  const isFresh = (row: T): boolean => {
    const seenAt = Date.parse(String((row as Record<string, unknown>)[lastSeenField]));
    return Number.isFinite(seenAt) && Date.now() - seenAt <= staleMs;
  };

  const recompute = () => {
    const next = new Set<string>();
    for (const row of rows.values()) {
      if (isFresh(row)) next.add(row.id);
    }
    if (next.size === online.size && [...next].every((id) => online.has(id))) return;
    online = next;
    for (const listener of listeners) listener(online);
  };

  recompute();

  const unsubscribeRealtime = await collection.subscribe<T>(
    "*",
    (event: RecordSubscription<T>) => {
      if (event.action === "delete") {
        rows.delete(event.record.id);
      } else {
        rows.set(event.record.id, event.record);
      }
      recompute();
    },
  );

  const heartbeatTimer = setInterval(() => {
    if (!ownId) return;
    collection.update(ownId, beat()).catch(() => {
      // A missed heartbeat (network blip, momentary auth expiry) just
      // means this row goes stale on schedule like any other peer's —
      // nothing to recover here beyond trying again next tick.
    });
  }, heartbeatMs);

  // Sweeps rows that have gone stale without a delete event (the
  // common case: the peer's tab closed or lost network, so no message
  // ever arrives) — checked at the same cadence as the heartbeat, which
  // bounds the worst-case detection lag to `heartbeatMs + staleMs`.
  const sweepTimer = setInterval(recompute, heartbeatMs);

  return {
    get online() {
      return online;
    },
    subscribe(callback) {
      listeners.add(callback);
      callback(online);
      return () => listeners.delete(callback);
    },
    async stop() {
      clearInterval(heartbeatTimer);
      clearInterval(sweepTimer);
      listeners.clear();
      await unsubscribeRealtime();
    },
  };
}
