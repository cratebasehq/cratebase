import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { CollectionModel } from "@cratebase/client";
import type { FieldSchema } from "@/lib/field-types";
import { SchemaFieldRow } from "@/components/collections/schema-field-row";

interface SortableFieldRowProps {
  field: FieldSchema;
  collections: CollectionModel[];
  onChange: (field: FieldSchema) => void;
  onRemove: () => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  nameError?: string | null;
  optionsError?: string | null;
}

/** Drag-and-drop wrapper around `SchemaFieldRow`. The grip handle is the
 * only draggable surface — every input/select/checkbox in the row stays
 * fully interactive instead of being swallowed by the drag gesture. The
 * dragged row is lifted (scaled + shadow, via `SchemaFieldRow`'s
 * `isDragging` styling) and every other row leaves a gap where it would
 * land, via dnd-kit's own transform — that gap is the drop indicator. */
export function SortableFieldRow({
  field,
  collections,
  onChange,
  onRemove,
  onMoveUp,
  onMoveDown,
  expanded,
  onExpandedChange,
  nameError,
  optionsError,
}: SortableFieldRowProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: field.id });

  return (
    <div
      ref={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
        zIndex: isDragging ? 10 : undefined,
        opacity: isDragging ? 0.92 : 1,
      }}
    >
      <SchemaFieldRow
        field={field}
        collections={collections}
        onChange={onChange}
        onRemove={onRemove}
        onMoveUp={onMoveUp}
        onMoveDown={onMoveDown}
        expanded={expanded}
        onExpandedChange={onExpandedChange}
        dragHandleAttributes={attributes}
        dragHandleListeners={listeners}
        isDragging={isDragging}
        nameError={nameError}
        optionsError={optionsError}
      />
    </div>
  );
}
