import { useEffect, useRef, useState } from "react";
import { Plus } from "lucide-react";
import type { CollectionModel } from "pocketbase";
import { type FieldSchema, newField, userFields } from "@/lib/field-types";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { SortableContext, arrayMove, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { SortableFieldRow } from "@/components/collections/sortable-field-row";
import { RuleField } from "@/components/collections/rule-field";

export interface CollectionFormValue {
  name: string;
  type: "base" | "auth";
  identityField: string;
  schema: FieldSchema[];
  listRule: string | null;
  viewRule: string | null;
  createRule: string | null;
  updateRule: string | null;
  deleteRule: string | null;
}

const NAME_RE = /^[a-zA-Z_][a-zA-Z0-9_]*$/;

/** How long an error waits before it appears while the field is still
 * being typed in. Matches the old inline-validation component: nothing is
 * shown until the field has been blurred once, and after that a newly
 * introduced error settles in rather than flashing on every keystroke. */
const VALIDATION_DEBOUNCE_MS = 400;

/** A text field that validates itself the way the deleted
 * `InlineValidation` did — quiet until first blur, then debounced while
 * typing and immediate on blur — built out of the shadcn `Field` set. */
function ValidatedTextField({
  label,
  value,
  onChange,
  validate,
  hint,
  placeholder,
  disabled = false,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  validate: (value: string) => string | null;
  hint?: string;
  placeholder?: string;
  disabled?: boolean;
}) {
  const [touched, setTouched] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // `validate` is an inline closure at every call site, so it can't be an
  // effect dependency without restarting the debounce on every render.
  const check = useRef(validate);
  useEffect(() => {
    check.current = validate;
  });

  useEffect(() => {
    if (!touched) return;
    const next = check.current(value);
    if (next === null) {
      setError(null);
      return;
    }
    const timer = setTimeout(() => setError(next), VALIDATION_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [value, touched]);

  return (
    <Field data-invalid={error !== null ? true : undefined}>
      <FieldLabel htmlFor="collection-name">{label}</FieldLabel>
      <Input
        id="collection-name"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={() => {
          setTouched(true);
          setError(check.current(value));
        }}
        placeholder={placeholder}
        disabled={disabled}
        aria-invalid={error !== null || undefined}
        className="h-control-md"
      />
      {error === null && hint ? <FieldDescription>{hint}</FieldDescription> : null}
      {error !== null ? <FieldError>{error}</FieldError> : null}
    </Field>
  );
}

/// Mirrors `_collections.name TEXT NOT NULL UNIQUE` (case-sensitive,
/// SQLite's default BINARY collation — "Posts" and "posts" do NOT
/// collide server-side, so this check doesn't lowercase-normalize
/// either). Without this, a colliding name only surfaced as a raw
/// "value for 'name' must be unique" toast after the save round-trip
/// instead of inline, right where the name is typed.
function validateName(value: string, otherCollections: CollectionModel[]): string | null {
  if (value.length === 0) return "Name is required";
  if (!NAME_RE.test(value)) return "Letters, digits, underscore; can't start with a digit";
  if (otherCollections.some((c) => c.name === value)) {
    return "Another collection already uses this name";
  }
  return null;
}

// Mirrors `cratebase_core::field::RESERVED_FIELD_NAMES` — these columns
// are managed by the server and can't be redefined as schema fields.
const RESERVED_FIELD_NAMES = ["id", "created", "updated", "collectionId", "collectionName", "expand"];

/** Field name errors, shown inline next to the field's name input rather
 * than as a form-wide toast. Mirrors the constraints
 * `cratebase_core::field::is_valid_identifier` and `RESERVED_FIELD_NAMES`
 * enforce server-side, so a bad name is caught before the save round-trip. */
function validateFieldName(field: FieldSchema, schema: FieldSchema[]): string | null {
  if (field.name.length === 0) return "Field name is required";
  if (field.name.length > 64) return "Field name must be 64 characters or fewer";
  if (!NAME_RE.test(field.name)) return "Letters, digits, underscore; can't start with a digit";
  if (RESERVED_FIELD_NAMES.includes(field.name)) return `"${field.name}" is a reserved field name`;
  if (schema.some((other) => other.id !== field.id && other.name === field.name)) {
    return "Another field already uses this name";
  }
  return null;
}

/** Type-specific option errors, shown inline inside the offending field's
 * options panel. Only checks constraints the API would otherwise reject
 * on save (a missing relation target, an empty select, an inverted
 * min/max range) — everything else is genuinely optional. */
function validateFieldOptions(field: FieldSchema): string | null {
  if (field.type === "relation" && !field.collectionId) return "Choose a target collection";
  if (field.type === "select" && ((field.values as string[] | undefined)?.length ?? 0) === 0) {
    return "Add at least one option value";
  }
  if (field.type === "autodate" && !field.onCreate && !field.onUpdate) {
    return 'Enable "Set on create" or "Set on update"';
  }
  if (["text", "editor", "password", "number"].includes(field.type)) {
    const min = field.min as number | undefined;
    const max = field.max as number | undefined;
    if (typeof min === "number" && typeof max === "number" && min > max) {
      return field.type === "number" ? "Min value can't exceed max value" : "Min length can't exceed max length";
    }
  }
  return null;
}

export function emptyCollectionForm(type: "base" | "auth" = "base"): CollectionFormValue {
  return {
    name: "",
    type,
    identityField: "email",
    schema: [],
    listRule: null,
    viewRule: null,
    createRule: null,
    updateRule: null,
    deleteRule: null,
  };
}

export function collectionToFormValue(collection: CollectionModel): CollectionFormValue {
  return {
    name: collection.name,
    type: collection.type === "auth" ? "auth" : "base",
    identityField: (collection.type === "auth" ? collection.passwordAuth?.identityFields?.[0] : undefined) ?? "email",
    schema: userFields(collection),
    listRule: collection.listRule ?? null,
    viewRule: collection.viewRule ?? null,
    createRule: collection.createRule ?? null,
    updateRule: collection.updateRule ?? null,
    deleteRule: collection.deleteRule ?? null,
  };
}

interface CollectionFormProps {
  value: CollectionFormValue;
  onChange: (value: CollectionFormValue) => void;
  otherCollections: CollectionModel[];
  isNew: boolean;
}

export function CollectionForm({ value, onChange, otherCollections, isNew }: CollectionFormProps) {
  function addField() {
    onChange({
      ...value,
      schema: [
        ...value.schema,
        newField({ id: crypto.randomUUID(), name: "", type: "text" }),
      ],
    });
  }

  function updateField(index: number, field: FieldSchema) {
    const schema = [...value.schema];
    schema[index] = field;
    onChange({ ...value, schema });
  }

  function removeField(index: number) {
    onChange({ ...value, schema: value.schema.filter((_, i) => i !== index) });
  }

  function moveField(index: number, direction: -1 | 1) {
    const target = index + direction;
    if (target < 0 || target >= value.schema.length) return;
    const schema = [...value.schema];
    [schema[index], schema[target]] = [schema[target], schema[index]];
    onChange({ ...value, schema });
  }

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }), useSensor(KeyboardSensor));

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const from = value.schema.findIndex((f) => f.id === active.id);
    const to = value.schema.findIndex((f) => f.id === over.id);
    if (from < 0 || to < 0) return;
    onChange({ ...value, schema: arrayMove(value.schema, from, to) });
  }


  return (
    <div className="flex flex-col gap-6">
      <ValidatedTextField
        label="Name"
        value={value.name}
        onChange={(name) => onChange({ ...value, name })}
        validate={(name) => validateName(name, otherCollections)}
        placeholder="posts"
        disabled={!isNew}
        hint={isNew ? undefined : "Renaming an existing collection isn't supported yet"}
      />

      {value.type === "auth" ? (
        <div>
          {isNew ? (
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              spacing={0}
              aria-label="Log in with"
              value={value.identityField}
              // A segmented control always has exactly one option picked —
              // Radix reports "" when the pressed item is toggled off.
              onValueChange={(identityField) => {
                if (identityField) onChange({ ...value, identityField });
              }}
            >
              <ToggleGroupItem value="email">Email</ToggleGroupItem>
              <ToggleGroupItem value="username">Username</ToggleGroupItem>
            </ToggleGroup>
          ) : (
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              spacing={0}
              aria-label="Log in with"
              value={value.identityField}
            >
              <ToggleGroupItem value={value.identityField} disabled>
                {value.identityField}
              </ToggleGroupItem>
            </ToggleGroup>
          )}
          {!isNew ? (
            <p className="mt-1.5 text-xs text-muted-foreground">Changing the identity field after creation isn't supported yet.</p>
          ) : null}
        </div>
      ) : null}

      <div>
        <div className="mb-2 flex items-center justify-between">
          <span className="text-sm font-medium text-foreground">Fields</span>
          <button
            type="button"
            onClick={addField}
            className="flex items-center gap-1 rounded-md px-2 py-1 text-sm font-medium text-primary transition-colors hover:bg-accent"
          >
            <Plus className="size-3.5" />
            Add field
          </button>
        </div>
        {value.type === "auth" ? (
          <p className="mb-2 text-xs text-muted-foreground">
            <code className="font-mono">{value.identityField}</code> and <code className="font-mono">password</code> are managed automatically for auth collections.
          </p>
        ) : null}
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <SortableContext items={value.schema.map((f) => f.id)} strategy={verticalListSortingStrategy}>
            <div className="flex flex-col gap-2">
              {value.schema.map((field, index) => (
                <SortableFieldRow
                  key={field.id}
                  field={field}
                  collections={otherCollections}
                  onChange={(next) => updateField(index, next)}
                  onRemove={() => removeField(index)}
                  onMoveUp={index > 0 ? () => moveField(index, -1) : undefined}
                  onMoveDown={index < value.schema.length - 1 ? () => moveField(index, 1) : undefined}
                  nameError={field.name.length > 0 ? validateFieldName(field, value.schema) : null}
                  optionsError={validateFieldOptions(field)}
                />
              ))}
              {value.schema.length === 0 ? (
                <p className="rounded-xl border border-dashed border-border p-4 text-center text-sm text-muted-foreground">
                  No fields yet.
                </p>
              ) : null}
            </div>
          </SortableContext>
        </DndContext>
      </div>

      <div className="flex flex-col gap-4">
        <span className="text-sm font-medium text-foreground">API rules</span>
        <RuleField label="List / Search" value={value.listRule} onChange={(listRule) => onChange({ ...value, listRule })} />
        <RuleField label="View" value={value.viewRule} onChange={(viewRule) => onChange({ ...value, viewRule })} />
        <RuleField label="Create" value={value.createRule} onChange={(createRule) => onChange({ ...value, createRule })} />
        <RuleField label="Update" value={value.updateRule} onChange={(updateRule) => onChange({ ...value, updateRule })} />
        <RuleField label="Delete" value={value.deleteRule} onChange={(deleteRule) => onChange({ ...value, deleteRule })} />
      </div>
    </div>
  );
}
