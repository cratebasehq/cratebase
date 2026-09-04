import type { CardRecord } from "../types";
import { useRegisterFlipNode } from "../hooks/useFlip";

export function CardItem({
  card,
  dragging,
  onDragStart,
  onDragEnd,
  onDelete,
}: {
  card: CardRecord;
  dragging: boolean;
  onDragStart: (event: React.DragEvent) => void;
  onDragEnd: () => void;
  onDelete: () => void;
}) {
  const registerNode = useRegisterFlipNode(card.id);

  return (
    <li
      ref={registerNode}
      className={`kanban-card ${dragging ? "dragging" : ""}`}
      draggable
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      data-id={card.id}
    >
      <span className="kanban-card-title">{card.title}</span>
      <button type="button" className="kanban-card-delete" aria-label="Delete card" onClick={onDelete}>
        ×
      </button>
    </li>
  );
}
