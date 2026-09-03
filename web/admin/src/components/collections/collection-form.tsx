import { Plus } from "lucide-react";
import type { CollectionModel, FieldSchema } from "cratebase";
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
import { InlineValidation } from "@/components/interior/inline-validation";
import { SegmentedControl } from "@/components/interior/segmented-control";
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

function validateName(value: string): string | null {
  if (value.length === 0) return "Name is required";
  if (!NAME_RE.test(value)) return "Letters, digits, underscore; can't start with a digit";
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
  const options = field.options ?? {};
  if (field.type === "relation" && !options.collectionId) return "Choose a target collection";
  if (field.type === "select" && ((options.values as string[] | undefined)?.length ?? 0) === 0) {
    return "Add at least one option value";
  }
  if (field.type === "autodate" && !options.onCreate && !options.onUpdate) {
    return 'Enable "Set on create" or "Set on update"';
  }
  if (["text", "editor", "password", "number"].includes(field.type)) {
    const min = options.min as number | undefined;
    const max = options.max as number | undefined;
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
    identityField: (collection.authOptions?.identityField as string | undefined) ?? "email",
    schema: collection.schema,
    listRule: collection.listRule,
    viewRule: collection.viewRule,
    createRule: collection.createRule,
    updateRule: collection.updateRule,
    deleteRule: collection.deleteRule,
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
        { id: crypto.randomUUID(), name: "", type: "text", required: false, unique: false, options: {} },
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
      <InlineValidation
        label="Name"
        value={value.name}
        onChange={(name) => onChange({ ...value, name })}
        validate={validateName}
        placeholder="posts"
        disabled={!isNew}
        hint={isNew ? undefined : "Renaming an existing collection isn't supported yet"}
      />

      {value.type === "auth" ? (
        <div>
          {isNew ? (
            <SegmentedControl
              label="Log in with"
              value={value.identityField}
              onValueChange={(identityField) => onChange({ ...value, identityField })}
              options={[
                { value: "email", label: "Email" },
                { value: "username", label: "Username" },
              ]}
            />
          ) : (
            <SegmentedControl
              label="Log in with"
              value={value.identityField}
              onValueChange={() => {}}
              options={[{ value: value.identityField, label: value.identityField, disabled: true }]}
            />
          )}
          {!isNew ? (
            <p className="mt-1.5 text-[11.5px] text-muted-foreground">Changing the identity field after creation isn't supported yet.</p>
          ) : null}
        </div>
      ) : null}

      <div>
        <div className="mb-2 flex items-center justify-between">
          <span className="text-[13px] font-medium text-foreground">Fields</span>
          <button
            type="button"
            onClick={addField}
            className="flex items-center gap-1 rounded-md px-2 py-1 text-[12px] font-medium text-primary transition-colors hover:bg-accent"
          >
            <Plus className="size-3.5" />
            Add field
          </button>
        </div>
        {value.type === "auth" ? (
          <p className="mb-2 text-[11.5px] text-muted-foreground">
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
                <p className="rounded-xl border border-dashed border-border p-4 text-center text-[12.5px] text-muted-foreground">
                  No fields yet.
                </p>
              ) : null}
            </div>
          </SortableContext>
        </DndContext>
      </div>

      <div className="flex flex-col gap-4">
        <span className="text-[13px] font-medium text-foreground">API rules</span>
        <RuleField label="List / Search" value={value.listRule} onChange={(listRule) => onChange({ ...value, listRule })} />
        <RuleField label="View" value={value.viewRule} onChange={(viewRule) => onChange({ ...value, viewRule })} />
        <RuleField label="Create" value={value.createRule} onChange={(createRule) => onChange({ ...value, createRule })} />
        <RuleField label="Update" value={value.updateRule} onChange={(updateRule) => onChange({ ...value, updateRule })} />
        <RuleField label="Delete" value={value.deleteRule} onChange={(deleteRule) => onChange({ ...value, deleteRule })} />
      </div>
    </div>
  );
}
