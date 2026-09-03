import { AlertCircle, GripVertical, Trash2 } from "lucide-react";
import type { CollectionModel, FieldSchema } from "cratebase";
import { FIELD_TYPES } from "@/lib/field-types";
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
  nameError?: string | null;
  optionsError?: string | null;
}

/** A single labeled control inside an `OptionGroup`, with helper text
 * underneath explaining what the constraint does — every option here maps
 * 1:1 to a key `cratebase_core::field::FieldOptions` reads, so the copy
 * doubles as inline documentation for that struct. */
function OptionField({
  label,
  help,
  children,
}: {
  label: string;
  help?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11.5px] font-medium text-foreground/80">{label}</span>
      {children}
      {help ? <span className="text-[10.5px] leading-snug text-muted-foreground">{help}</span> : null}
    </label>
  );
}

/** A titled, visually distinct panel grouping the options relevant to one
 * field type, so a `select` field never shows `relation`/`file` controls
 * and vice versa. */
function OptionGroup({
  title,
  error,
  children,
}: {
  title: string;
  error?: string | null;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-2.5 rounded-lg border border-border/60 bg-secondary/30 p-2.5">
      <div className="flex items-center justify-between gap-2">
        <p className="text-[10.5px] font-semibold uppercase tracking-wide text-muted-foreground/70">{title}</p>
        {error ? (
          <span className="flex items-center gap-1 text-[10.5px] font-medium text-destructive">
            <AlertCircle className="size-3" />
            {error}
          </span>
        ) : null}
      </div>
      {children}
    </div>
  );
}

function NumberInput({
  value,
  onChange,
  placeholder,
}: {
  value: number | undefined;
  onChange: (value: number | undefined) => void;
  placeholder?: string;
}) {
  return (
    <input
      type="number"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value === "" ? undefined : Number(e.target.value))}
      placeholder={placeholder}
      className="h-8 w-full rounded-lg border border-border bg-secondary/60 px-2 text-[12.5px] text-foreground outline-none focus:border-primary"
    />
  );
}

function MultipleValuesCheckbox({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
}) {
  return (
    <label className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="size-3.5 accent-primary"
      />
      {label}
    </label>
  );
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
  nameError,
  optionsError,
}: SchemaFieldRowProps) {
  const options = field.options ?? {};
  const multiple = (options.multiple as boolean | undefined) ?? false;

  function patch(patchOptions: Record<string, unknown>) {
    onChange({ ...field, options: { ...options, ...patchOptions } });
  }

  return (
    <div
      className={`flex flex-col gap-2.5 rounded-xl border bg-card/60 p-3 transition-shadow ${
        isDragging
          ? "border-primary shadow-[0_8px_24px_-12px_rgba(0,0,0,0.4)]"
          : nameError
            ? "border-destructive/50"
            : "border-border"
      }`}
    >
      <div
        className={`flex items-center gap-2 rounded-lg transition-colors ${
          isDragging ? "bg-primary/[0.06]" : ""
        }`}
      >
        <button
          type="button"
          onKeyDown={(e) => {
            if (e.key === "ArrowUp") onMoveUp?.();
            if (e.key === "ArrowDown") onMoveDown?.();
          }}
          className={`group grid size-7 shrink-0 cursor-grab touch-none place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground active:cursor-grabbing ${
            isDragging ? "bg-primary/10 text-primary" : ""
          }`}
          aria-label="Drag to reorder, or use the arrow keys"
          title="Drag to reorder, or focus and use the arrow keys"
          {...dragHandleAttributes}
          {...dragHandleListeners}
        >
          <GripVertical className="size-4 transition-transform group-hover:scale-110" />
        </button>

        <div className="min-w-0 flex-1">
          <input
            type="text"
            value={field.name}
            onChange={(e) => onChange({ ...field, name: e.target.value })}
            placeholder="field_name"
            aria-invalid={nameError ? true : undefined}
            className={`h-8 w-full rounded-lg border bg-secondary/60 px-2 font-mono text-[12.5px] text-foreground outline-none ${
              nameError ? "border-destructive/60 focus:border-destructive" : "border-border focus:border-primary"
            }`}
          />
        </div>

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

      {nameError ? (
        <p className="-mt-1 flex items-center gap-1 pl-9 text-[11px] font-medium text-destructive">
          <AlertCircle className="size-3 shrink-0" />
          {nameError}
        </p>
      ) : null}

      {field.type === "text" || field.type === "editor" || field.type === "password" ? (
        <OptionGroup title="Length & format" error={optionsError}>
          <div className="grid grid-cols-2 gap-2">
            <OptionField label="Min length" help="Minimum character count. Leave blank for no minimum.">
              <NumberInput
                value={options.min as number | undefined}
                onChange={(min) => patch({ min })}
                placeholder="No minimum"
              />
            </OptionField>
            <OptionField label="Max length" help="Maximum character count. Leave blank for no maximum.">
              <NumberInput
                value={options.max as number | undefined}
                onChange={(max) => patch({ max })}
                placeholder="No maximum"
              />
            </OptionField>
          </div>
          <OptionField label="Pattern" help="A regular expression the value must fully match. Leave blank to skip.">
            <input
              type="text"
              value={(options.pattern as string | undefined) ?? ""}
              onChange={(e) => patch({ pattern: e.target.value || undefined })}
              placeholder="e.g. ^[a-z0-9-]+$"
              className="h-8 w-full rounded-lg border border-border bg-secondary/60 px-2 font-mono text-[12px] text-foreground outline-none focus:border-primary"
            />
          </OptionField>
        </OptionGroup>
      ) : null}

      {field.type === "number" ? (
        <OptionGroup title="Range & precision" error={optionsError}>
          <div className="grid grid-cols-2 gap-2">
            <OptionField label="Min value" help="Reject values below this. Leave blank for no minimum.">
              <NumberInput
                value={options.min as number | undefined}
                onChange={(min) => patch({ min })}
                placeholder="No minimum"
              />
            </OptionField>
            <OptionField label="Max value" help="Reject values above this. Leave blank for no maximum.">
              <NumberInput
                value={options.max as number | undefined}
                onChange={(max) => patch({ max })}
                placeholder="No maximum"
              />
            </OptionField>
          </div>
          <label className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
            <input
              type="checkbox"
              checked={(options.onlyInt as boolean | undefined) ?? false}
              onChange={(e) => patch({ onlyInt: e.target.checked })}
              className="size-3.5 accent-primary"
            />
            Integer only — reject decimal values
          </label>
        </OptionGroup>
      ) : null}

      {field.type === "select" ? (
        <OptionGroup title="Choices" error={optionsError}>
          <TagInput
            label="Allowed values"
            value={(options.values as string[] | undefined) ?? []}
            onChange={(values) => patch({ values })}
            placeholder="Add an option and press Enter"
            hint="Records can only store one of these values per selection"
          />
          <MultipleValuesCheckbox
            checked={multiple}
            onChange={(checked) => patch({ multiple: checked })}
            label="Allow multiple selections"
          />
          {multiple ? (
            <OptionField label="Max selections" help="Cap how many values can be selected at once. Leave blank for no cap.">
              <NumberInput
                value={options.maxSelect as number | undefined}
                onChange={(maxSelect) => patch({ maxSelect })}
                placeholder="Unlimited"
              />
            </OptionField>
          ) : null}
        </OptionGroup>
      ) : null}

      {field.type === "relation" ? (
        <OptionGroup title="Relation target" error={optionsError}>
          <OptionField
            label="Target collection"
            help="Values stored here must be the id of an existing record in this collection."
          >
            <select
              value={(options.collectionId as string | undefined) ?? ""}
              onChange={(e) => patch({ collectionId: e.target.value || undefined })}
              className="h-8 w-full rounded-lg border border-border bg-secondary/60 px-2 text-[12.5px] text-foreground outline-none focus:border-primary"
            >
              <option value="">Select target collection…</option>
              {collections.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </OptionField>
          <MultipleValuesCheckbox
            checked={multiple}
            onChange={(checked) => patch({ multiple: checked })}
            label="Allow multiple related records"
          />
        </OptionGroup>
      ) : null}

      {field.type === "file" ? (
        <OptionGroup title="File constraints" error={optionsError}>
          <TagInput
            label="Allowed MIME types"
            value={(options.mimeTypes as string[] | undefined) ?? []}
            onChange={(mimeTypes) => patch({ mimeTypes })}
            placeholder="e.g. image/png"
            hint="Leave empty to allow any file type"
          />
          <OptionField label="Max file size (bytes)" help="Reject uploads larger than this. e.g. 5242880 = 5 MB. Leave blank for no limit.">
            <NumberInput
              value={options.maxSize as number | undefined}
              onChange={(maxSize) => patch({ maxSize })}
              placeholder="No limit"
            />
          </OptionField>
          <MultipleValuesCheckbox
            checked={multiple}
            onChange={(checked) => patch({ multiple: checked })}
            label="Allow multiple files"
          />
        </OptionGroup>
      ) : null}

      {field.type === "autodate" ? (
        <OptionGroup title="Timing" error={optionsError}>
          <div className="flex items-center gap-4">
            <label className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
              <input
                type="checkbox"
                checked={(options.onCreate as boolean | undefined) ?? false}
                onChange={(e) => patch({ onCreate: e.target.checked })}
                className="size-3.5 accent-primary"
              />
              Set on create
            </label>
            <label className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
              <input
                type="checkbox"
                checked={(options.onUpdate as boolean | undefined) ?? false}
                onChange={(e) => patch({ onUpdate: e.target.checked })}
                className="size-3.5 accent-primary"
              />
              Set on update
            </label>
          </div>
          <p className="text-[10.5px] leading-snug text-muted-foreground">
            The value is computed by the server; clients can't set it. At least one of the two must be enabled.
          </p>
        </OptionGroup>
      ) : null}
    </div>
  );
}
