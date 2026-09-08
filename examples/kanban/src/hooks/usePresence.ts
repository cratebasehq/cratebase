import { useEffect, useState } from "react";
import { PRESENCE_COLLECTION, cb } from "../cratebase";
import type { AuthUser } from "./useAuth";
import type { PresenceRecord } from "../types";

const HEARTBEAT_MS = 8000;
const STALE_AFTER_MS = 20000;

/** Presence tracking approach (see README's "How it works" for the full
 * rationale): there is no server-side connection registry to query, so
 * presence is approximated client-side with a `presence` collection —
 * one record per user, upserted on a heartbeat interval. Every tab
 * subscribes to `presence` realtime events (so a peer's arrival/update is
 * near-instant) and additionally treats any record whose `lastSeen` has
 * gone stale as offline, which is what covers a tab closing without
 * firing `beforeunload` (crash, network drop, killed process). */
export function usePresence(user: AuthUser | null) {
  const [peers, setPeers] = useState<Map<string, PresenceRecord>>(new Map());

  useEffect(() => {
    if (!user) {
      setPeers(new Map());
      return;
    }

    let cancelled = false;
    let ownRecordId = localStorage.getItem(`kanban:presenceId:${user.id}`) || null;

    async function upsertOwnPresence(): Promise<PresenceRecord> {
      const name = user!.email || user!.id;
      try {
        if (ownRecordId) {
          return (await cb.collection(PRESENCE_COLLECTION).update(ownRecordId, { name })) as unknown as PresenceRecord;
        }
        throw new Error("no existing presence record");
      } catch {
        const record = (await cb
          .collection(PRESENCE_COLLECTION)
          .create({ userId: user!.id, name })) as unknown as PresenceRecord;
        ownRecordId = record.id;
        localStorage.setItem(`kanban:presenceId:${user!.id}`, record.id);
        return record;
      }
    }

    async function boot() {
      const initial = (await cb.collection(PRESENCE_COLLECTION).fullList()) as unknown as PresenceRecord[];
      if (cancelled) return;
      setPeers(new Map(initial.map((r) => [r.userId, r])));

      // Merge our own upserted record in immediately instead of waiting on
      // the realtime round trip — subscribing only starts below, so the
      // "create" event for this very upsert would otherwise go unheard and
      // we'd read as offline to ourselves until the next heartbeat.
      try {
        const own = await upsertOwnPresence();
        if (!cancelled) setPeers((prev) => new Map(prev).set(own.userId, own));
      } catch (err) {
        console.error("presence upsert failed", err);
      }

      await cb
        .collection(PRESENCE_COLLECTION)
        .subscribe("*", (event) => {
          const record = event.record as unknown as PresenceRecord;
          setPeers((prev) => {
            const next = new Map(prev);
            if (event.action === "delete") {
              next.delete(record.userId);
            } else {
              next.set(record.userId, record);
            }
            return next;
          });
        })
        .catch((err) => console.error("presence subscribe failed", err));
    }

    boot();

    const heartbeat = window.setInterval(() => {
      upsertOwnPresence().catch((err) => console.error("presence heartbeat failed", err));
    }, HEARTBEAT_MS);

    // Best-effort: covers a normal tab close/navigation. A crashed tab or
    // dropped network instead relies on the staleness check below.
    const onUnload = () => {
      if (!ownRecordId) return;
      navigator.sendBeacon?.(
        `${cb.buildURL(`/api/collections/${PRESENCE_COLLECTION}/records/${ownRecordId}`)}`,
        new Blob([], { type: "application/json" }),
      );
    };
    window.addEventListener("beforeunload", onUnload);

    return () => {
      cancelled = true;
      window.clearInterval(heartbeat);
      window.removeEventListener("beforeunload", onUnload);
    };
  }, [user]);

  const now = usePresenceClock();
  const online = new Map<string, PresenceRecord>();
  for (const [userId, record] of peers) {
    if (now - new Date(record.lastSeen).getTime() < STALE_AFTER_MS) {
      online.set(userId, record);
    }
  }
  return online;
}

/** Ticks once a second so the staleness filter above re-evaluates even
 * when no realtime event arrives — otherwise a peer whose tab crashed
 * would stay listed as online forever. */
function usePresenceClock(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  return now;
}
