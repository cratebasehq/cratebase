import { Fragment, useRef, useState } from "react";
import type { CardRecord, Status } from "../types";
import { STATUS_LABEL } from "../types";
import { CardItem } from "./CardItem";

export function Column({
  status,
  cards,
  draggedId,
  isDropTarget,
  dropIndex,
  onHover,
  onDragStart,
  onDragEnd,
  onDrop,
  onDelete,
  onCreate,
}: {
  status: Status;
  cards: CardRecord[];
  draggedId: string | null;
  isDropTarget: boolean;
  dropIndex: number | null;
  onHover: (status: Status, index: number) => void;
  onDragStart: (card: CardRecord, event: React.DragEvent) => void;
  onDragEnd: () => void;
  onDrop: () => void;
  onDelete: (id: string) => void;
  onCreate: (title: string) => void;
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
    onHover(status, index);
  }

  function handleDrop(event: React.DragEvent) {
    event.preventDefault();
    onDrop();
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
    <section className="column" data-status={status} onDragOver={handleDragOver} onDrop={handleDrop}>
      <header className="column-header">
        <span className="column-dot" aria-hidden="true" />
        <h2>{STATUS_LABEL[status]}</h2>
        <span className="column-count">{cards.length}</span>
      </header>

      <ul className="kanban-list" ref={listRef}>
        {cards.map((card, index) => (
          <Fragment key={card.id}>
            {isDropTarget && dropIndex === index && <li className="drop-indicator" />}
            <CardItem
              card={card}
              dragging={draggedId === card.id}
              onDragStart={(event) => onDragStart(card, event)}
              onDragEnd={onDragEnd}
              onDelete={() => onDelete(card.id)}
            />
          </Fragment>
        ))}
        {isDropTarget && dropIndex === cards.length && <li className="drop-indicator" key="indicator-end" />}
      </ul>

      {composing ? (
        <form className="card-composer" onSubmit={submitCreate}>
          <input
            autoFocus
            placeholder="Card title…"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onBlur={() => {
              if (!title.trim()) setComposing(false);
            }}
          />
          <div className="card-composer-actions">
            <button type="submit" className="btn btn-primary btn-sm">
              Add
            </button>
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => setComposing(false)}>
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <button type="button" className="add-card-btn" onClick={() => setComposing(true)}>
          + Add a card
        </button>
      )}
    </section>
  );
}
