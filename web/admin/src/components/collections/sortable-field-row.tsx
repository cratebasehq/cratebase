import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { CollectionModel, FieldSchema } from "cratebase";
import { SchemaFieldRow } from "@/components/collections/schema-field-row";

interface SortableFieldRowProps {
  field: FieldSchema;
  collections: CollectionModel[];
  onChange: (field: FieldSchema) => void;
  onRemove: () => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
}

/** Drag-and-drop wrapper around `SchemaFieldRow`. The grip handle is the
 * only draggable surface — every input/select/checkbox in the row stays
 * fully interactive instead of being swallowed by the drag gesture. */
export function SortableFieldRow({ field, collections, onChange, onRemove, onMoveUp, onMoveDown }: SortableFieldRowProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: field.id });

  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition, zIndex: isDragging ? 10 : undefined }}
    >
      <SchemaFieldRow
        field={field}
        collections={collections}
        onChange={onChange}
        onRemove={onRemove}
        onMoveUp={onMoveUp}
        onMoveDown={onMoveDown}
        dragHandleAttributes={attributes}
        dragHandleListeners={listeners}
        isDragging={isDragging}
      />
    </div>
  );
}
