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

export function emptyCollectionForm(): CollectionFormValue {
  return {
    name: "",
    type: "base",
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
      <div className="flex items-end gap-3">
        <div className="flex-1">
          <InlineValidation
            label="Name"
            value={value.name}
            onChange={(name) => onChange({ ...value, name })}
            validate={validateName}
            placeholder="posts"
            disabled={!isNew}
            hint={isNew ? undefined : "Renaming an existing collection isn't supported yet"}
          />
        </div>
        <SegmentedControl
          label="Collection type"
          value={value.type}
          onValueChange={(type) => onChange({ ...value, type: type as "base" | "auth" })}
          options={[
            { value: "base", label: "Base", disabled: !isNew },
            { value: "auth", label: "Auth", disabled: !isNew },
          ]}
        />
      </div>

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
            <code className="font-mono">email</code> and <code className="font-mono">password</code> are managed automatically for auth collections.
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
