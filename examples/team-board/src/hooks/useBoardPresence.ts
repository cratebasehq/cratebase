// Per-team presence ("who's viewing this board right now"). Hand-rolled
// directly on `@cratebase/client`'s CRUD + realtime, the same pattern
// examples/kanban/src/hooks/usePresence.ts documents and live-verified:
// there is no server-side "who's connected" registry
// (sdk/js/react/src/usePresence.ts's module doc says as much about
// `@cratebase/react`'s own generic `usePresence`), so this keeps one
// `presence` row per (team, user), heartbeats it, and treats a stale
// `lastSeenAt` as offline (covers a crashed tab / dropped network that
// never fires `beforeunload`). Not `@cratebase/react`'s `usePresence`
// because that hook only exposes the *set of online ids*, not each
// peer's own fields (name) — this board wants to render real names/
// avatars, not just a count.
import { useEffect, useState } from "react";
import { cb } from "../cratebase.js";
import type { PresenceRecord } from "../cratebase-types.js";

const HEARTBEAT_MS = 8000;
const STALE_AFTER_MS = 20000;

export function useBoardPresence(teamId: string | null, userId: string | undefined, name: string | undefined) {
  const [peers, setPeers] = useState<Map<string, PresenceRecord>>(new Map());

  useEffect(() => {
    if (!teamId || !userId) {
      setPeers(new Map());
      return;
    }

    let cancelled = false;
    const storageKey = `team-board:presenceId:${teamId}:${userId}`;
    let ownRecordId = localStorage.getItem(storageKey);

    async function upsertOwn(): Promise<PresenceRecord> {
      const lastSeenAt = new Date().toISOString();
      if (ownRecordId) {
        try {
          return await cb.collection("presence").update(ownRecordId, { name: name ?? "", lastSeenAt });
        } catch {
          // Fell out of sync with the server (row deleted, e.g. by another
          // tab's cleanup) — fall through and create a fresh one below.
        }
      }
      const record = await cb.collection("presence").create({ teamRef: teamId!, userRef: userId!, name: name ?? "", lastSeenAt });
      ownRecordId = record.id;
      localStorage.setItem(storageKey, record.id);
      return record;
    }

    async function boot() {
      const initial = await cb.collection("presence").fullList({ filter: `teamRef = "${teamId}"` });
      if (cancelled) return;
      setPeers(new Map(initial.map((r) => [r.userRef, r])));

      try {
        const own = await upsertOwn();
        if (!cancelled) setPeers((prev) => new Map(prev).set(own.userRef, own));
      } catch (err) {
        console.error("presence upsert failed", err);
      }

      await cb
        .collection("presence")
        .subscribe(
          "*",
          (event) => {
            const record = event.record;
            setPeers((prev) => {
              const next = new Map(prev);
              if (event.action === "delete") next.delete(record.userRef);
              else next.set(record.userRef, record);
              return next;
            });
          },
          { filter: `teamRef = "${teamId}"` },
        )
        .catch((err) => console.error("presence subscribe failed", err));
    }

    boot();

    const heartbeat = window.setInterval(() => {
      upsertOwn().catch((err) => console.error("presence heartbeat failed", err));
    }, HEARTBEAT_MS);

    const onUnload = () => {
      if (!ownRecordId) return;
      navigator.sendBeacon?.(cb.buildURL(`/api/collections/presence/records/${ownRecordId}`), new Blob([], { type: "application/json" }));
    };
    window.addEventListener("beforeunload", onUnload);

    return () => {
      cancelled = true;
      window.clearInterval(heartbeat);
      window.removeEventListener("beforeunload", onUnload);
    };
  }, [teamId, userId, name]);

  const now = usePresenceClock();
  const online: PresenceRecord[] = [];
  for (const record of peers.values()) {
    if (now - new Date(record.lastSeenAt).getTime() < STALE_AFTER_MS) online.push(record);
  }
  return online;
}

function usePresenceClock(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  return now;
}
