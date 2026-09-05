import { useId } from "react";
import { AlertCircle, ChevronRight, GripVertical, Trash2 } from "lucide-react";
import type { CollectionModel } from "pocketbase";
import { cn } from "@/lib/utils";
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
  /** Whether the type-specific options panel is showing. Owned by the form
   * so only the row you're working on is open. */
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
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
  className,
}: {
  label: string;
  help?: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <label className={cn("flex min-w-0 flex-col gap-1", className)}>
      <span className="text-xs font-medium text-foreground/80">{label}</span>
      {children}
      {help ? <span className="text-2xs leading-snug text-muted-foreground">{help}</span> : null}
    </label>
  );
}

/** A titled panel grouping the options relevant to one field type, so a
 * `select` field never shows `relation`/`file` controls and vice versa. */
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
    <div className="flex flex-col gap-3 border-t border-border bg-surface-sunken/60 px-3 py-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-2xs font-semibold uppercase tracking-wider text-muted-foreground/70">{title}</p>
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
      <label htmlFor={id} className="cursor-pointer text-xs text-muted-foreground">
        {label}
      </label>
    </div>
  );
}

/** True when the field's type has anything to configure at all — a `bool`
 * has no options, so it gets no disclosure arrow. */
function hasOptions(type: FieldSchema["type"]): boolean {
  return type !== "bool";
}

/** The one-line version of the options panel, shown on the collapsed row so
 * a 20-field schema can be read without opening 20 disclosures. */
function optionsSummary(field: FieldSchema, collections: CollectionModel[]): string {
  const parts: string[] = [];
  const min = field.min as number | undefined;
  const max = field.max as number | undefined;
  switch (field.type) {
    case "text":
    case "editor":
    case "password":
      if (typeof min === "number" && min > 0) parts.push(`min ${min}`);
      if (typeof max === "number" && max > 0) parts.push(`max ${max}`);
      if (field.pattern) parts.push("pattern");
      break;
    case "number":
      if (typeof min === "number") parts.push(`≥ ${min}`);
      if (typeof max === "number") parts.push(`≤ ${max}`);
      if (field.onlyInt) parts.push("integer");
      break;
    case "select": {
      const values = (field.values as string[] | undefined) ?? [];
      parts.push(values.length === 0 ? "no values" : values.slice(0, 3).join(", ") + (values.length > 3 ? "…" : ""));
      if (isMultiValue(field)) parts.push(`up to ${field.maxSelect}`);
      break;
    }
    case "relation": {
      const target = collections.find((c) => c.id === field.collectionId);
      parts.push(target ? `→ ${target.name}` : "no target");
      if (isMultiValue(field)) parts.push(`up to ${field.maxSelect}`);
      break;
    }
    case "file": {
      const mimes = (field.mimeTypes as string[] | undefined) ?? [];
      parts.push(mimes.length > 0 ? mimes.slice(0, 2).join(", ") : "any type");
      if (field.maxSize) parts.push(`≤ ${Math.round(Number(field.maxSize) / 1024)} KB`);
      if (isMultiValue(field)) parts.push(`up to ${field.maxSelect}`);
      break;
    }
    case "autodate":
      if (field.onCreate) parts.push("on create");
      if (field.onUpdate) parts.push("on update");
      break;
    case "vector": {
      const dimensions = field.dimensions as number | undefined;
      parts.push(dimensions ? `${dimensions}d` : "no dimensions set");
      const embedding = field.embedding as { sourceField?: string } | undefined;
      if (embedding?.sourceField) parts.push(`auto from ${embedding.sourceField}`);
      break;
    }
    default:
      break;
  }
  return parts.join(" · ");
}

export function SchemaFieldRow({
  field,
  collections,
  onChange,
  onRemove,
  onMoveUp,
  onMoveDown,
  expanded,
  onExpandedChange,
  dragHandleAttributes,
  dragHandleListeners,
  isDragging,
  nameError,
  optionsError,
}: SchemaFieldRowProps) {
  const multiple = isMultiValue(field);
  const expandable = hasOptions(field.type);
  const open = expanded && expandable;

  // select/relation/file express "allow more than one" as `maxSelect > 1`
  // rather than a boolean, flat on the field object — there's no `options`
  // wrapper on the wire.
  function patch(patch: Record<string, unknown>) {
    onChange({ ...field, ...patch } as FieldSchema);
  }

  function toggleMultiple(checked: boolean) {
    patch({ maxSelect: checked ? Math.max((field.maxSelect as number | undefined) ?? 0, 2) : 1 });
  }

  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border bg-card transition-shadow",
        isDragging
          ? "border-primary shadow-e3"
          : nameError || optionsError
            ? "border-destructive/50"
            : "border-border",
      )}
    >
      <div className="flex items-center gap-2 px-2 py-2">
        <button
          type="button"
          onKeyDown={(e) => {
            if (e.key === "ArrowUp") onMoveUp?.();
            if (e.key === "ArrowDown") onMoveDown?.();
          }}
          className={cn(
            "group grid size-control-sm shrink-0 cursor-grab touch-none place-items-center rounded-md text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground active:cursor-grabbing",
            isDragging && "bg-accent text-foreground",
          )}
          aria-label="Drag to reorder, or use the arrow keys"
          title="Drag to reorder, or focus and use the arrow keys"
          {...dragHandleAttributes}
          {...dragHandleListeners}
        >
          <GripVertical className="size-4" />
        </button>

        <Input
          type="text"
          value={field.name}
          onChange={(e) => onChange({ ...field, name: e.target.value })}
          placeholder="field_name"
          aria-label="Field name"
          aria-invalid={nameError ? true : undefined}
          className="h-control-md min-w-0 max-w-80 flex-1 font-mono text-sm"
        />

        <Select
          value={field.type}
          onValueChange={(type) => {
            // Type-specific settings (min/max, values, collectionId, ...)
            // don't carry over to a different type — start that type fresh.
            onChange(
              newField({
                id: field.id,
                name: field.name,
                required: field.required,
                type: type as FieldSchema["type"],
              }),
            );
            onExpandedChange(hasOptions(type as FieldSchema["type"]));
          }}
        >
          <SelectTrigger aria-label="Field type" className="h-control-md w-32 shrink-0 text-sm">
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

        {/* The collapsed row still has to say what the field does; the open
            one keeps the same spacer so nothing shifts when it opens. */}
        <span className="hidden min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground/80 xl:block">
          {open ? "" : optionsSummary(field, collections)}
        </span>

        <CheckboxOption
          checked={field.required ?? false}
          onChange={(required) => onChange({ ...field, required })}
          label="Required"
          className="shrink-0"
        />

        {expandable ? (
          <button
            type="button"
            onClick={() => onExpandedChange(!expanded)}
            aria-expanded={open}
            aria-label={open ? "Hide options" : "Show options"}
            className="grid size-control-sm shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ChevronRight className={cn("size-4 transition-transform", open && "rotate-90")} />
          </button>
        ) : (
          <span className="size-control-sm shrink-0" />
        )}

        <button
          type="button"
          onClick={onRemove}
          className="grid size-control-sm shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
          aria-label={`Remove ${field.name || "field"}`}
        >
          <Trash2 className="size-3.5" />
        </button>
      </div>

      {nameError ? (
        <p className="flex items-center gap-1 px-3 pb-2 pl-11 text-xs font-medium text-destructive">
          <AlertCircle className="size-3 shrink-0" />
          {nameError}
        </p>
      ) : null}

      {open && (field.type === "text" || field.type === "editor" || field.type === "password") ? (
        <OptionGroup title="Length & format" error={optionsError}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
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
            <OptionField label="Pattern" help="A regular expression the value must fully match.">
              <Input
                type="text"
                value={(field.pattern as string | undefined) ?? ""}
                onChange={(e) => patch({ pattern: e.target.value || undefined })}
                placeholder="e.g. ^[a-z0-9-]+$"
                className="h-control-md font-mono text-sm"
              />
            </OptionField>
          </div>
        </OptionGroup>
      ) : null}

      {open && field.type === "number" ? (
        <OptionGroup title="Range & precision" error={optionsError}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
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
            <div className="flex items-end pb-5">
              <CheckboxOption
                checked={(field.onlyInt as boolean | undefined) ?? false}
                onChange={(onlyInt) => patch({ onlyInt })}
                label="Integer only — reject decimals"
              />
            </div>
          </div>
        </OptionGroup>
      ) : null}

      {open && field.type === "select" ? (
        <OptionGroup title="Choices" error={optionsError}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <TagInput
              label="Allowed values"
              value={(field.values as string[] | undefined) ?? []}
              onChange={(values) => patch({ values })}
              placeholder="Add an option and press Enter"
              hint="Records can only store one of these values per selection"
            />
            <div className="flex flex-col gap-3">
              <CheckboxOption checked={multiple} onChange={toggleMultiple} label="Allow multiple selections" />
              {multiple ? (
                <OptionField label="Max selections" help="Cap how many values can be selected at once.">
                  <NumberInput
                    value={field.maxSelect as number | undefined}
                    onChange={(maxSelect) => patch({ maxSelect: maxSelect ?? 2 })}
                    placeholder="e.g. 3"
                  />
                </OptionField>
              ) : null}
            </div>
          </div>
        </OptionGroup>
      ) : null}

      {open && field.type === "relation" ? (
        <OptionGroup title="Relation target" error={optionsError}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
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
            <div className="flex flex-col gap-3">
              <CheckboxOption checked={multiple} onChange={toggleMultiple} label="Allow multiple related records" />
              {multiple ? (
                <OptionField label="Max related records" help="Cap how many records can be related at once.">
                  <NumberInput
                    value={field.maxSelect as number | undefined}
                    onChange={(maxSelect) => patch({ maxSelect: maxSelect ?? 2 })}
                    placeholder="e.g. 3"
                  />
                </OptionField>
              ) : null}
            </div>
          </div>
        </OptionGroup>
      ) : null}

      {open && field.type === "file" ? (
        <OptionGroup title="File constraints" error={optionsError}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <TagInput
              label="Allowed MIME types"
              value={(field.mimeTypes as string[] | undefined) ?? []}
              onChange={(mimeTypes) => patch({ mimeTypes })}
              placeholder="e.g. image/png"
              hint="Leave empty to allow any file type"
            />
            <div className="flex flex-col gap-3">
              <OptionField
                label="Max file size (bytes)"
                help="Reject uploads larger than this. e.g. 5242880 = 5 MB. Leave blank for no limit."
              >
                <NumberInput
                  value={field.maxSize as number | undefined}
                  onChange={(maxSize) => patch({ maxSize })}
                  placeholder="No limit"
                />
              </OptionField>
              <CheckboxOption checked={multiple} onChange={toggleMultiple} label="Allow multiple files" />
              {multiple ? (
                <OptionField label="Max files" help="Cap how many files can be uploaded at once.">
                  <NumberInput
                    value={field.maxSelect as number | undefined}
                    onChange={(maxSelect) => patch({ maxSelect: maxSelect ?? 2 })}
                    placeholder="e.g. 3"
                  />
                </OptionField>
              ) : null}
            </div>
          </div>
        </OptionGroup>
      ) : null}

      {open && field.type === "autodate" ? (
        <OptionGroup title="Timing" error={optionsError}>
          <div className="flex items-center gap-6">
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

      {open && field.type === "vector" ? (
        <OptionGroup title="Vector" error={optionsError}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <OptionField label="Dimensions" help="How many floats each stored vector must have. Fixed once records exist.">
              <NumberInput
                value={field.dimensions as number | undefined}
                onChange={(dimensions) => patch({ dimensions: dimensions ?? 0 })}
                placeholder="e.g. 1536"
              />
            </OptionField>
            <div className="flex items-end pb-5 sm:col-span-2">
              <CheckboxOption
                checked={field.embedding != null}
                onChange={(checked) =>
                  patch({
                    embedding: checked
                      ? { provider: "echo", model: "", sourceField: "" }
                      : null,
                  })
                }
                label="Compute automatically from another field on save, instead of accepting the array directly"
              />
            </div>
          </div>
          {field.embedding != null ? (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
              <OptionField label="Provider" help='"echo" is a deterministic, network-free test provider. Anything else uses the HTTP provider configured on the server.'>
                <Input
                  type="text"
                  value={((field.embedding as { provider?: string } | undefined)?.provider as string | undefined) ?? ""}
                  onChange={(e) =>
                    patch({ embedding: { ...(field.embedding as object), provider: e.target.value } })
                  }
                  placeholder="echo"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
              <OptionField label="Model" help="Passed to the HTTP provider. Ignored by the echo provider.">
                <Input
                  type="text"
                  value={((field.embedding as { model?: string } | undefined)?.model as string | undefined) ?? ""}
                  onChange={(e) => patch({ embedding: { ...(field.embedding as object), model: e.target.value } })}
                  placeholder="text-embedding-3-small"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
              <OptionField label="Source field" help="Name of the text field on this record whose value is embedded on save.">
                <Input
                  type="text"
                  value={
                    ((field.embedding as { sourceField?: string } | undefined)?.sourceField as string | undefined) ??
                    ""
                  }
                  onChange={(e) =>
                    patch({ embedding: { ...(field.embedding as object), sourceField: e.target.value } })
                  }
                  placeholder="body"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
            </div>
          ) : null}
        </OptionGroup>
      ) : null}

      {open &&
      (field.type === "email" ||
        field.type === "url" ||
        field.type === "date" ||
        field.type === "json" ||
        field.type === "geoPoint") ? (
        <OptionGroup title="Options" error={optionsError}>
          <p className="text-2xs leading-snug text-muted-foreground">
            {field.type === "email"
              ? "Validated as an email address. Domain allow/deny lists are set through the API."
              : field.type === "url"
                ? "Validated as an absolute URL. Domain allow/deny lists are set through the API."
                : field.type === "date"
                  ? "Stored as a UTC timestamp. Filter it with the date macros — @now, @todayStart, @monthStart."
                  : field.type === "json"
                    ? "Stored as JSON. Filterable with :length and :each; not sortable."
                    : "Stored as { lon, lat } in decimal degrees. Longitude in [-180, 180], latitude in [-90, 90]."}
          </p>
        </OptionGroup>
      ) : null}
    </div>
  );
}
