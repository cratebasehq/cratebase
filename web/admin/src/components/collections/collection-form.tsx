import { useEffect, useMemo, useRef, useState } from "react";
import { Plus } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { type FieldSchema, newField } from "@/lib/field-types";
import {
  validateFieldName,
  validateFieldOptions,
  validateName,
  type CollectionFormValue,
} from "@/lib/collection-form-value";
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
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { SortableFieldRow } from "@/components/collections/sortable-field-row";
import { RuleField } from "@/components/collections/rule-field";
import { IndexEditor } from "@/components/collections/index-editor";
import { AuthOptionsEditor } from "@/components/collections/auth-options-editor";

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

interface CollectionFormProps {
  value: CollectionFormValue;
  onChange: (value: CollectionFormValue) => void;
  otherCollections: CollectionModel[];
  isNew: boolean;
  /** Field names that already exist on the server. Deleting one of these
   * drops a real column and its data; deleting a field added in this
   * session doesn't. */
  persistedFieldNames?: ReadonlySet<string>;
}

export function CollectionForm({
  value,
  onChange,
  otherCollections,
  isNew,
  persistedFieldNames,
}: CollectionFormProps) {
  /** Only one options panel open at a time — the point of the disclosure is
   * that a 20-field schema stays readable. */
  const [expandedFieldId, setExpandedFieldId] = useState<string | null>(null);
  const [pendingRemoval, setPendingRemoval] = useState<{ index: number; field: FieldSchema } | null>(null);

  function addField() {
    const field = newField({ id: crypto.randomUUID(), name: "", type: "text" });
    onChange({ ...value, schema: [...value.schema, field] });
    setExpandedFieldId(field.id);
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
    [schema[index], schema[target]] = [schema[target]!, schema[index]!];
    onChange({ ...value, schema });
  }

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor),
  );

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const from = value.schema.findIndex((f) => f.id === active.id);
    const to = value.schema.findIndex((f) => f.id === over.id);
    if (from < 0 || to < 0) return;
    onChange({ ...value, schema: arrayMove(value.schema, from, to) });
  }

  /** Everything an index may be built on: the server's own columns plus
   * whatever this form currently defines. */
  const indexColumns = useMemo(() => {
    const base = ["id", "created", "updated"];
    if (value.type === "auth") base.push(value.identityField, "verified");
    return [...base, ...value.schema.map((f) => f.name).filter(Boolean)];
  }, [value.schema, value.type, value.identityField]);

  const dropsColumn = pendingRemoval ? (persistedFieldNames?.has(pendingRemoval.field.name) ?? false) : false;

  return (
    <div className="flex flex-col gap-8">
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
            <p className="mt-1.5 text-xs text-muted-foreground">
              Changing the identity field after creation isn't supported yet.
            </p>
          ) : null}
        </div>
      ) : null}

      <section className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <div className="flex flex-col">
            <span className="text-sm font-medium text-foreground">Fields</span>
            {value.type === "auth" ? (
              <span className="text-xs text-muted-foreground">
                <code className="font-mono">{value.identityField}</code> and{" "}
                <code className="font-mono">password</code> are managed automatically.
              </span>
            ) : null}
          </div>
          <button
            type="button"
            onClick={addField}
            className="flex h-control-sm items-center gap-1.5 rounded-md border border-border px-2 text-sm font-medium transition-colors hover:bg-accent"
          >
            <Plus className="size-3.5" />
            Add field
          </button>
        </div>

        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <SortableContext items={value.schema.map((f) => f.id)} strategy={verticalListSortingStrategy}>
            <div className="flex flex-col gap-2">
              {value.schema.map((field, index) => (
                <SortableFieldRow
                  key={field.id}
                  field={field}
                  collections={otherCollections}
                  onChange={(next) => updateField(index, next)}
                  onRemove={() => setPendingRemoval({ index, field })}
                  onMoveUp={index > 0 ? () => moveField(index, -1) : undefined}
                  onMoveDown={index < value.schema.length - 1 ? () => moveField(index, 1) : undefined}
                  expanded={expandedFieldId === field.id}
                  onExpandedChange={(open) => setExpandedFieldId(open ? field.id : null)}
                  nameError={field.name.length > 0 ? validateFieldName(field, value.schema) : null}
                  optionsError={validateFieldOptions(field)}
                />
              ))}
              {value.schema.length === 0 ? (
                <p className="rounded-lg border border-dashed border-border p-4 text-center text-sm text-muted-foreground">
                  No fields yet.
                </p>
              ) : null}
            </div>
          </SortableContext>
        </DndContext>
      </section>

      {value.type === "auth" && value.auth ? (
        <AuthOptionsEditor
          value={value.auth}
          onChange={(auth) => onChange({ ...value, auth })}
          collectionName={value.name}
        />
      ) : null}

      <section className="flex flex-col gap-3">
        <div className="flex flex-col">
          <span className="text-sm font-medium text-foreground">API rules</span>
          <span className="text-xs text-muted-foreground">
            A filter expression evaluated per request. Empty means public; unset means superusers only.
          </span>
        </div>
        <RuleField label="List / Search" value={value.listRule} onChange={(listRule) => onChange({ ...value, listRule })} />
        <RuleField label="View" value={value.viewRule} onChange={(viewRule) => onChange({ ...value, viewRule })} />
        <RuleField label="Create" value={value.createRule} onChange={(createRule) => onChange({ ...value, createRule })} />
        <RuleField label="Update" value={value.updateRule} onChange={(updateRule) => onChange({ ...value, updateRule })} />
        <RuleField label="Delete" value={value.deleteRule} onChange={(deleteRule) => onChange({ ...value, deleteRule })} />
      </section>

      <IndexEditor
        collectionName={value.name || "collection"}
        columnOptions={indexColumns}
        value={value.indexes}
        onChange={(indexes) => onChange({ ...value, indexes })}
      />

      <AlertDialog open={pendingRemoval !== null} onOpenChange={(open) => !open && setPendingRemoval(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Remove <span className="font-mono">{pendingRemoval?.field.name || "this field"}</span>?
            </AlertDialogTitle>
            <AlertDialogDescription>
              {dropsColumn ? (
                <>
                  This field exists on the server. Saving after this removes its column from{" "}
                  <span className="font-mono">{value.name}</span> and every value stored in it, for every record.
                  That cannot be undone.
                </>
              ) : (
                <>This field hasn't been saved yet, so nothing stored is lost.</>
              )}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant={dropsColumn ? "destructive" : "default"}
              onClick={() => {
                if (pendingRemoval) removeField(pendingRemoval.index);
                setPendingRemoval(null);
              }}
            >
              {dropsColumn ? "Remove field and its data" : "Remove field"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
