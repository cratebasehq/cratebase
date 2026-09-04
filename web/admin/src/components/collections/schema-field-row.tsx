import { useId } from "react";
import { AlertCircle, GripVertical, Trash2 } from "lucide-react";
import type { CollectionModel } from "pocketbase";
import { FIELD_TYPES, type FieldSchema, isMultiValue, newField } from "@/lib/field-types";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { TagInput } from "@/components/ui/tag-input";

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
      <span className="text-xs font-medium text-foreground/80">{label}</span>
      {children}
      {help ? <span className="text-2xs leading-snug text-muted-foreground">{help}</span> : null}
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
        <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground/70">{title}</p>
        {error ? (
          <span className="flex items-center gap-1 text-2xs font-medium text-destructive">
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
    <Input
      type="number"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value === "" ? undefined : Number(e.target.value))}
      placeholder={placeholder}
      className="h-control-md text-sm"
    />
  );
}

/** One boolean field option. Radix's checkbox is a `<button>`, so the
 * label is wired up with `htmlFor` rather than by wrapping it. */
function CheckboxOption({
  checked,
  onChange,
  label,
  className = "",
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  className?: string;
}) {
  const id = useId();
  return (
    <div className={`flex items-center gap-1.5 ${className}`}>
      <Checkbox id={id} checked={checked} onCheckedChange={(next) => onChange(next === true)} />
      <label htmlFor={id} className="text-xs text-muted-foreground">
        {label}
      </label>
    </div>
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
  const multiple = isMultiValue(field);

  // select/relation/file express "allow more than one" as `maxSelect > 1`
  // rather than a boolean, flat on the field object — there's no `options`
  // wrapper on the wire.
  function patch(patch: Record<string, unknown>) {
    onChange({ ...field, ...patch } as FieldSchema);
  }

  function toggleMultiple(checked: boolean) {
    patch({ maxSelect: checked ? Math.max(field.maxSelect as number | undefined ?? 0, 2) : 1 });
  }

  return (
    <div
      className={`flex flex-col gap-2.5 rounded-xl border bg-card/60 p-3 transition-shadow ${
        isDragging
          ? "border-primary shadow-e3"
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
          <Input
            type="text"
            value={field.name}
            onChange={(e) => onChange({ ...field, name: e.target.value })}
            placeholder="field_name"
            aria-label="Field name"
            aria-invalid={nameError ? true : undefined}
            className="h-control-md font-mono text-sm"
          />
        </div>

        <Select
          value={field.type}
          onValueChange={(type) =>
            // Type-specific settings (min/max, values, collectionId, ...) don't
            // carry over to a different type — start that type fresh.
            onChange(newField({ id: field.id, name: field.name, required: field.required, type: type as FieldSchema["type"] }))
          }
        >
          <SelectTrigger aria-label="Field type" className="h-control-md shrink-0 text-sm">
            <SelectValue placeholder="Type…" />
          </SelectTrigger>
          <SelectContent>
            {FIELD_TYPES.map((t) => (
              <SelectItem key={t.value} value={t.value}>
                {t.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <CheckboxOption
          checked={field.required ?? false}
          onChange={(required) => onChange({ ...field, required })}
          label="Required"
          className="shrink-0"
        />

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
        <p className="-mt-1 flex items-center gap-1 pl-9 text-xs font-medium text-destructive">
          <AlertCircle className="size-3 shrink-0" />
          {nameError}
        </p>
      ) : null}

      {field.type === "text" || field.type === "editor" || field.type === "password" ? (
        <OptionGroup title="Length & format" error={optionsError}>
          <div className="grid grid-cols-2 gap-2">
            <OptionField label="Min length" help="Minimum character count. Leave blank for no minimum.">
              <NumberInput
                value={field.min as number | undefined}
                onChange={(min) => patch({ min })}
                placeholder="No minimum"
              />
            </OptionField>
            <OptionField label="Max length" help="Maximum character count. Leave blank for no maximum.">
              <NumberInput
                value={field.max as number | undefined}
                onChange={(max) => patch({ max })}
                placeholder="No maximum"
              />
            </OptionField>
          </div>
          <OptionField label="Pattern" help="A regular expression the value must fully match. Leave blank to skip.">
            <Input
              type="text"
              value={(field.pattern as string | undefined) ?? ""}
              onChange={(e) => patch({ pattern: e.target.value || undefined })}
              placeholder="e.g. ^[a-z0-9-]+$"
              className="h-control-md font-mono text-sm"
            />
          </OptionField>
        </OptionGroup>
      ) : null}

      {field.type === "number" ? (
        <OptionGroup title="Range & precision" error={optionsError}>
          <div className="grid grid-cols-2 gap-2">
            <OptionField label="Min value" help="Reject values below this. Leave blank for no minimum.">
              <NumberInput
                value={field.min as number | undefined}
                onChange={(min) => patch({ min })}
                placeholder="No minimum"
              />
            </OptionField>
            <OptionField label="Max value" help="Reject values above this. Leave blank for no maximum.">
              <NumberInput
                value={field.max as number | undefined}
                onChange={(max) => patch({ max })}
                placeholder="No maximum"
              />
            </OptionField>
          </div>
          <CheckboxOption
            checked={(field.onlyInt as boolean | undefined) ?? false}
            onChange={(onlyInt) => patch({ onlyInt })}
            label="Integer only — reject decimal values"
          />
        </OptionGroup>
      ) : null}

      {field.type === "select" ? (
        <OptionGroup title="Choices" error={optionsError}>
          <TagInput
            label="Allowed values"
            value={(field.values as string[] | undefined) ?? []}
            onChange={(values) => patch({ values })}
            placeholder="Add an option and press Enter"
            hint="Records can only store one of these values per selection"
          />
          <CheckboxOption
            checked={multiple}
            onChange={toggleMultiple}
            label="Allow multiple selections"
          />
          {multiple ? (
            <OptionField label="Max selections" help="Cap how many values can be selected at once.">
              <NumberInput
                value={field.maxSelect as number | undefined}
                onChange={(maxSelect) => patch({ maxSelect: maxSelect ?? 2 })}
                placeholder="e.g. 3"
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
            <Select
              value={(field.collectionId as string | undefined) ?? ""}
              onValueChange={(collectionId) => patch({ collectionId: collectionId || undefined })}
            >
              <SelectTrigger aria-label="Target collection" className="h-control-md w-full text-sm">
                <SelectValue placeholder="Select target collection…" />
              </SelectTrigger>
              <SelectContent>
                {collections.map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </OptionField>
          <CheckboxOption
            checked={multiple}
            onChange={toggleMultiple}
            label="Allow multiple related records"
          />
          {multiple ? (
            <OptionField label="Max related records" help="Cap how many records can be related at once.">
              <NumberInput
                value={field.maxSelect as number | undefined}
                onChange={(maxSelect) => patch({ maxSelect: maxSelect ?? 2 })}
                placeholder="e.g. 3"
              />
            </OptionField>
          ) : null}
        </OptionGroup>
      ) : null}

      {field.type === "file" ? (
        <OptionGroup title="File constraints" error={optionsError}>
          <TagInput
            label="Allowed MIME types"
            value={(field.mimeTypes as string[] | undefined) ?? []}
            onChange={(mimeTypes) => patch({ mimeTypes })}
            placeholder="e.g. image/png"
            hint="Leave empty to allow any file type"
          />
          <OptionField label="Max file size (bytes)" help="Reject uploads larger than this. e.g. 5242880 = 5 MB. Leave blank for no limit.">
            <NumberInput
              value={field.maxSize as number | undefined}
              onChange={(maxSize) => patch({ maxSize })}
              placeholder="No limit"
            />
          </OptionField>
          <CheckboxOption
            checked={multiple}
            onChange={toggleMultiple}
            label="Allow multiple files"
          />
          {multiple ? (
            <OptionField label="Max files" help="Cap how many files can be uploaded at once.">
              <NumberInput
                value={field.maxSelect as number | undefined}
                onChange={(maxSelect) => patch({ maxSelect: maxSelect ?? 2 })}
                placeholder="e.g. 3"
              />
            </OptionField>
          ) : null}
        </OptionGroup>
      ) : null}

      {field.type === "autodate" ? (
        <OptionGroup title="Timing" error={optionsError}>
          <div className="flex items-center gap-4">
            <CheckboxOption
              checked={(field.onCreate as boolean | undefined) ?? false}
              onChange={(onCreate) => patch({ onCreate })}
              label="Set on create"
            />
            <CheckboxOption
              checked={(field.onUpdate as boolean | undefined) ?? false}
              onChange={(onUpdate) => patch({ onUpdate })}
              label="Set on update"
            />
          </div>
          <p className="text-2xs leading-snug text-muted-foreground">
            The value is computed by the server; clients can't set it. At least one of the two must be enabled.
          </p>
        </OptionGroup>
      ) : null}
    </div>
  );
}
