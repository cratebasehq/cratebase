import type { CollectionModel } from "pocketbase";
import { type FieldSchema, userFields } from "@/lib/field-types";

/** The editable shape of a collection, as the schema editor holds it —
 * everything `PATCH /api/collections/:id` accepts that this dashboard
 * exposes, and nothing else. */
export interface CollectionFormValue {
  name: string;
  type: "base" | "auth";
  identityField: string;
  schema: FieldSchema[];
  /** Raw `CREATE INDEX` statements, exactly as the collection stores them. */
  indexes: string[];
  listRule: string | null;
  viewRule: string | null;
  createRule: string | null;
  updateRule: string | null;
  deleteRule: string | null;
}

const NAME_RE = /^[a-zA-Z_][a-zA-Z0-9_]*$/;

/**
 * Mirrors `_collections.name TEXT NOT NULL UNIQUE` (case-sensitive,
 * SQLite's default BINARY collation — "Posts" and "posts" do NOT collide
 * server-side, so this check doesn't lowercase-normalize either). Without
 * it, a colliding name only surfaced as a raw "value for 'name' must be
 * unique" toast after the save round-trip instead of inline, right where
 * the name is typed.
 */
export function validateName(value: string, otherCollections: CollectionModel[]): string | null {
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
export function validateFieldName(field: FieldSchema, schema: FieldSchema[]): string | null {
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
 * options panel. Only checks constraints the API would otherwise reject on
 * save (a missing relation target, an empty select, an inverted min/max
 * range) — everything else is genuinely optional. */
export function validateFieldOptions(field: FieldSchema): string | null {
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

/**
 * Everything wrong with the form right now, as one list.
 *
 * The old editor showed these inline but still let Save through, so a
 * schema with a nameless field or a relation pointing nowhere was sent to
 * the server and came back as a raw error toast. Save is gated on this
 * being empty.
 */
export function collectionFormErrors(value: CollectionFormValue, otherCollections: CollectionModel[]): string[] {
  const errors: string[] = [];
  const nameError = validateName(value.name, otherCollections);
  if (nameError) errors.push(`Name: ${nameError.toLowerCase()}`);
  for (const field of value.schema) {
    const named = validateFieldName(field, value.schema);
    if (named) errors.push(`${field.name || "Unnamed field"}: ${named.toLowerCase()}`);
    const options = validateFieldOptions(field);
    if (options) errors.push(`${field.name || "Unnamed field"}: ${options.toLowerCase()}`);
  }
  return errors;
}

export function emptyCollectionForm(type: "base" | "auth" = "base"): CollectionFormValue {
  return {
    name: "",
    type,
    identityField: "email",
    schema: [],
    indexes: [],
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
    indexes: [...(collection.indexes ?? [])],
    listRule: collection.listRule ?? null,
    viewRule: collection.viewRule ?? null,
    createRule: collection.createRule ?? null,
    updateRule: collection.updateRule ?? null,
    deleteRule: collection.deleteRule ?? null,
  };
}
