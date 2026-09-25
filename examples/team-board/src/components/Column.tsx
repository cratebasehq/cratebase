import { Fragment, useRef, useState } from "react";
import type { CardsRecord, ColumnsRecord } from "../cratebase-types.js";
import { CardItem } from "./CardItem.js";

export function Column({
  column,
  cards,
  draggedId,
  isDropTarget,
  dropIndex,
  highlightIds,
  onHover,
  onDragStart,
  onDragEnd,
  onDrop,
  onCreate,
  onOpenCard,
}: {
  column: ColumnsRecord;
  cards: CardsRecord[];
  draggedId: string | null;
  isDropTarget: boolean;
  dropIndex: number | null;
  highlightIds: Set<string> | null;
  onHover: (columnId: string, index: number) => void;
  onDragStart: (card: CardsRecord, event: React.DragEvent) => void;
  onDragEnd: () => void;
  onDrop: () => void;
  onCreate: (title: string) => void;
  onOpenCard: (id: string) => void;
}) {
  const listRef = useRef<HTMLUListElement>(null);
  const [composing, setComposing] = useState(false);
  const [title, setTitle] = useState("");

  function handleDragOver(event: React.DragEvent) {
    event.preventDefault();
    const list = listRef.current;
    if (!list) return;
    const children = [...list.querySelectorAll<HTMLElement>("[data-id]")].filter((el) => el.dataset.id !== draggedId);
    let index = children.length;
    for (let i = 0; i < children.length; i++) {
      const rect = children[i].getBoundingClientRect();
      if (event.clientY < rect.top + rect.height / 2) {
        index = i;
        break;
      }
    }
    onHover(column.id, index);
  }

  function submitCreate(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = title.trim();
    if (!trimmed) return;
    onCreate(trimmed);
    setTitle("");
    setComposing(false);
  }

  return (
    <section
      className="flex w-72 shrink-0 flex-col rounded-panel border border-slate/50 bg-paper-100 dark:border-slate-dark dark:bg-ink-700"
      onDragOver={handleDragOver}
      onDrop={(e) => {
        e.preventDefault();
        onDrop();
      }}
    >
      <header className="flex items-center gap-2 border-b border-slate/40 px-3 py-2.5 dark:border-slate-dark">
        <h2 className="flex-1 truncate font-display text-sm font-semibold">{column.name}</h2>
        <span className="rounded-[4px] bg-paper-200 px-1.5 py-0.5 font-mono text-[11px] text-ink/60 dark:bg-ink-600 dark:text-paper/60">
          {cards.length}
        </span>
      </header>

      <ul ref={listRef} className="flex min-h-[8px] flex-1 flex-col gap-2 p-2">
        {cards.map((card, index) => (
          <Fragment key={card.id}>
            {isDropTarget && dropIndex === index && <li className="h-1 rounded-full bg-crate" />}
            <CardItem
              card={card}
              dragging={draggedId === card.id}
              dimmed={highlightIds !== null && !highlightIds.has(card.id)}
              onDragStart={(event) => onDragStart(card, event)}
              onDragEnd={onDragEnd}
              onOpen={() => onOpenCard(card.id)}
            />
          </Fragment>
        ))}
        {isDropTarget && dropIndex === cards.length && <li className="h-1 rounded-full bg-crate" />}
      </ul>

      {composing ? (
        <form className="flex flex-col gap-2 p-2 pt-0" onSubmit={submitCreate}>
          <input
            autoFocus
            className="input"
            placeholder="Card title…"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onBlur={() => {
              if (!title.trim()) setComposing(false);
            }}
          />
          <div className="flex gap-2">
            <button type="submit" className="btn-primary py-1 text-xs">
              Add
            </button>
            <button type="button" className="btn-ghost py-1 text-xs" onClick={() => setComposing(false)}>
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <button
          type="button"
          className="m-2 mt-0 rounded-control px-2 py-1.5 text-left text-sm text-ink/50 hover:bg-paper-200 dark:text-paper/50 dark:hover:bg-ink-600"
          onClick={() => setComposing(true)}
        >
          + Add a card
        </button>
      )}
    </section>
  );
}
