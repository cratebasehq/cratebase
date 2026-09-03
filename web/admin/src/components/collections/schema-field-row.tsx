import { GripVertical, Trash2 } from "lucide-react";
import type { CollectionModel, FieldSchema } from "cratebase";
import { FIELD_TYPES, MULTI_VALUE_TYPES } from "@/lib/field-types";
import { TagInput } from "@/components/interior/tag-input";

interface SchemaFieldRowProps {
  field: FieldSchema;
  collections: CollectionModel[];
  onChange: (field: FieldSchema) => void;
  onRemove: () => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  dragHandleAttributes?: React.HTMLAttributes<HTMLButtonElement>;
  dragHandleListeners?: Record<string, unknown>;
  isDragging?: boolean;
}

export function SchemaFieldRow({
  field,
  collections,
  onChange,
  onRemove,
  onMoveUp,
  onMoveDown,
  dragHandleAttributes,
  dragHandleListeners,
  isDragging,
}: SchemaFieldRowProps) {
  const options = field.options ?? {};
  const multiValue = MULTI_VALUE_TYPES.includes(field.type);

  function patch(patchOptions: Record<string, unknown>) {
    onChange({ ...field, options: { ...options, ...patchOptions } });
  }

  return (
    <div
      className={`flex flex-col gap-2.5 rounded-xl border bg-card/60 p-3 transition-shadow ${
        isDragging ? "border-primary shadow-[0_8px_24px_-12px_rgba(0,0,0,0.4)]" : "border-border"
      }`}
    >
      <div className="flex items-center gap-2">
        <button
          type="button"
          onKeyDown={(e) => {
            if (e.key === "ArrowUp") onMoveUp?.();
            if (e.key === "ArrowDown") onMoveDown?.();
          }}
          className="grid size-6 shrink-0 cursor-grab touch-none place-items-center text-muted-foreground transition-colors hover:text-foreground active:cursor-grabbing"
          aria-label="Drag to reorder, or use the arrow keys"
          {...dragHandleAttributes}
          {...dragHandleListeners}
        >
          <GripVertical className="size-3.5" />
        </button>

        <input
          type="text"
          value={field.name}
          onChange={(e) => onChange({ ...field, name: e.target.value })}
          placeholder="field_name"
          className="h-8 min-w-0 flex-1 rounded-lg border border-border bg-secondary/60 px-2 font-mono text-[12.5px] text-foreground outline-none focus:border-primary"
        />

        <select
          value={field.type}
          onChange={(e) => onChange({ ...field, type: e.target.value as FieldSchema["type"], options: {} })}
          className="h-8 shrink-0 rounded-lg border border-border bg-secondary/60 px-2 text-[12.5px] text-foreground outline-none focus:border-primary"
        >
          {FIELD_TYPES.map((t) => (
            <option key={t.value} value={t.value}>
              {t.label}
            </option>
          ))}
        </select>

        <label className="flex shrink-0 items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <input
            type="checkbox"
            checked={field.required ?? false}
            onChange={(e) => onChange({ ...field, required: e.target.checked })}
            className="size-3.5 accent-primary"
          />
          Required
        </label>
        <label className="flex shrink-0 items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <input
            type="checkbox"
            checked={field.unique ?? false}
            onChange={(e) => onChange({ ...field, unique: e.target.checked })}
            className="size-3.5 accent-primary"
          />
          Unique
        </label>

        <button
          type="button"
          onClick={onRemove}
          className="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
          aria-label="Remove field"
        >
          <Trash2 className="size-3.5" />
        </button>
      </div>

      {field.type === "select" ? (
        <TagInput
          label="Options"
          value={(options.values as string[] | undefined) ?? []}
          onChange={(values) => patch({ values })}
          placeholder="Add an option and press Enter"
        />
      ) : null}

      {field.type === "relation" ? (
        <select
          value={(options.collectionId as string | undefined) ?? ""}
          onChange={(e) => patch({ collectionId: e.target.value })}
          className="h-8 w-full rounded-lg border border-border bg-secondary/60 px-2 text-[12.5px] text-foreground outline-none focus:border-primary"
        >
          <option value="">Select target collection…</option>
          {collections.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      ) : null}

      {multiValue ? (
        <label className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <input
            type="checkbox"
            checked={(options.multiple as boolean | undefined) ?? false}
            onChange={(e) => patch({ multiple: e.target.checked })}
            className="size-3.5 accent-primary"
          />
          Allow multiple values
        </label>
      ) : null}

      {field.type === "text" || field.type === "editor" ? (
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={(options.pattern as string | undefined) ?? ""}
            onChange={(e) => patch({ pattern: e.target.value || undefined })}
            placeholder="Validation regex (optional)"
            className="h-8 w-full rounded-lg border border-border bg-secondary/60 px-2 font-mono text-[12px] text-foreground outline-none focus:border-primary"
          />
        </div>
      ) : null}

      {field.type === "number" ? (
        <label className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <input
            type="checkbox"
            checked={(options.onlyInt as boolean | undefined) ?? false}
            onChange={(e) => patch({ onlyInt: e.target.checked })}
            className="size-3.5 accent-primary"
          />
          Integers only
        </label>
      ) : null}
    </div>
  );
}
