import { useState } from "react";
import { useCards } from "../hooks/useCards";
import { usePresence } from "../hooks/usePresence";
import { useFlipOnChange, useFlipRegistry, FlipRegistryContext } from "../hooks/useFlip";
import type { AuthUser } from "../hooks/useAuth";
import { STATUSES } from "../types";
import type { CardRecord, Status } from "../types";
import { describeError } from "../pocketbase";
import { Column } from "./Column";
import { PresenceBar } from "./PresenceBar";
import { Avatar } from "./Avatar";

export function Board({ user, onLogout }: { user: AuthUser; onLogout: () => void }) {
  const { cards, loading, connected, createCard, deleteCard, moveCard } = useCards(true);
  const peers = usePresence(user);
  const [draggedId, setDraggedId] = useState<string | null>(null);
  const [hover, setHover] = useState<{ status: Status; index: number } | null>(null);
  const [error, setError] = useState("");

  const registry = useFlipRegistry();
  const flipKey = cards.map((c) => `${c.id}:${c.status}:${c.order}`).join("|");
  useFlipOnChange(registry, flipKey);

  const byStatus = new Map<Status, CardRecord[]>(STATUSES.map((s) => [s, []]));
  for (const card of cards) byStatus.get(card.status)!.push(card);
  for (const list of byStatus.values()) list.sort((a, b) => a.order - b.order);

  async function run<T>(action: () => Promise<T>) {
    try {
      await action();
      setError("");
    } catch (err) {
      setError(describeError(err));
    }
  }

  function handleDrop() {
    if (draggedId && hover) {
      const target = hover;
      run(() => moveCard(draggedId, target.status, target.index));
    }
    setDraggedId(null);
    setHover(null);
  }

  return (
    <FlipRegistryContext.Provider value={registry}>
      <div className="app-shell">
        <header className="app-header">
          <div className="brand">
            <span className="brand-mark" aria-hidden="true" />
            Cratebase Kanban
          </div>
          <div className="header-right">
            <PresenceBar peers={peers} selfUserId={user.id} />
            <span className={`realtime-pill ${connected ? "connected" : ""}`}>
              <span className="realtime-dot" />
              {connected ? "live" : "connecting…"}
            </span>
            <span className="whoami">
              <Avatar id={user.id} size={24} />
              {user.email}
            </span>
            <button type="button" className="btn btn-ghost btn-sm" onClick={onLogout}>
              Sign out
            </button>
          </div>
        </header>

        {error && <div className="banner">{error}</div>}

        <main className="board" aria-busy={loading}>
          {STATUSES.map((status) => (
            <Column
              key={status}
              status={status}
              cards={byStatus.get(status)!}
              draggedId={draggedId}
              isDropTarget={hover?.status === status && draggedId !== null}
              dropIndex={hover?.status === status ? hover.index : null}
              onHover={(s, index) => setHover({ status: s, index })}
              onDragStart={(card, event) => {
                setDraggedId(card.id);
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", card.id);
              }}
              onDragEnd={() => {
                setDraggedId(null);
                setHover(null);
              }}
              onDrop={handleDrop}
              onDelete={(id) => run(() => deleteCard(id))}
              onCreate={(title) => run(() => createCard(status, title))}
            />
          ))}
        </main>
      </div>
    </FlipRegistryContext.Provider>
  );
}
