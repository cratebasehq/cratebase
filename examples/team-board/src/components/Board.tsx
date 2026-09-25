import { useState } from "react";
import type { CardsRecord, ColumnsRecord } from "../cratebase-types.js";
import type { useCards } from "../hooks/useCards.js";
import { describeError } from "../cratebase.js";
import { Column } from "./Column.js";

export function Board({
  columns,
  cardsApi,
  highlightIds,
  searching,
  onOpenCard,
}: {
  teamId: string;
  columns: ColumnsRecord[];
  cardsApi: ReturnType<typeof useCards>;
  highlightIds: Set<string> | null;
  searching: boolean;
  onOpenCard: (id: string) => void;
}) {
  const { cards, createCard, moveCard } = cardsApi;
  const [draggedId, setDraggedId] = useState<string | null>(null);
  const [hover, setHover] = useState<{ columnId: string; index: number } | null>(null);
  const [error, setError] = useState("");

  const byColumn = new Map<string, CardsRecord[]>(columns.map((c) => [c.id, []]));
  for (const card of cards) {
    if (!card.columnRef) continue;
    byColumn.get(card.columnRef)?.push(card);
  }
  for (const list of byColumn.values()) list.sort((a, b) => a.order - b.order);

  async function run(action: () => Promise<unknown>) {
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
      run(() => moveCard(draggedId, target.columnId, target.index));
    }
    setDraggedId(null);
    setHover(null);
  }

  return (
    <div>
      {error && (
        <div className="mb-4 rounded-control border border-crate/30 bg-crate-50 px-3 py-2 text-sm text-crate dark:border-crate-dark/40 dark:bg-crate-dark/10 dark:text-crate-dark">
          {error}
        </div>
      )}
      {searching && <p className="mb-3 text-xs text-ink/50 dark:text-paper/50">Searching…</p>}
      <div className="flex items-start gap-4">
        {columns.map((column) => (
          <Column
            key={column.id}
            column={column}
            cards={byColumn.get(column.id) ?? []}
            draggedId={draggedId}
            isDropTarget={hover?.columnId === column.id && draggedId !== null}
            dropIndex={hover?.columnId === column.id ? hover.index : null}
            highlightIds={highlightIds}
            onHover={(columnId, index) => setHover({ columnId, index })}
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
            onCreate={(title) => run(() => createCard(column.id, title))}
            onOpenCard={onOpenCard}
          />
        ))}
      </div>
    </div>
  );
}
