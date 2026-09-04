import type { CollectionField, CollectionModel } from "pocketbase";

/** PocketBase's own `CollectionField` types every field as
 * `{ [key: string]: any; id, name, type: string, system, hidden, presentable }`
 * — it doesn't export a closed `FieldType` union, so the dashboard keeps its
 * own (it also drives the "add field" type picker below, which the wire
 * type has no equivalent of). */
export type FieldType =
  | "text"
  | "editor"
  | "number"
  | "bool"
  | "email"
  | "url"
  | "date"
  | "autodate"
  | "select"
  | "json"
  | "relation"
  | "file"
  | "password"
  | "geoPoint"
  | "vector";

export const FIELD_TYPES: { value: FieldType; label: string }[] = [
  { value: "text", label: "Text" },
  { value: "editor", label: "Editor" },
  { value: "number", label: "Number" },
  { value: "bool", label: "Bool" },
  { value: "email", label: "Email" },
  { value: "url", label: "URL" },
  { value: "date", label: "Date" },
  { value: "autodate", label: "Autodate" },
  { value: "select", label: "Select" },
  { value: "json", label: "JSON" },
  { value: "relation", label: "Relation" },
  { value: "file", label: "File" },
  { value: "password", label: "Password" },
  { value: "geoPoint", label: "Geo point" },
  { value: "vector", label: "Vector" },
];

/** A collection field, narrowed to this dashboard's `FieldType` union.
 * Every type-specific setting (`min`, `max`, `values`, `maxSelect`,
 * `collectionId`, ...) lives flat on the object, same as the wire format —
 * `CollectionField`'s own `[key: string]: any` index signature covers those,
 * this just tightens `type`. */
export type FieldSchema = CollectionField & { type: FieldType };

/** A new field, filled in with the same "nothing set yet" defaults the
 * server itself would use — for wherever this dashboard constructs one
 * client-side (adding a field, resetting one to a new type) rather than
 * reading one back from the API. */
export function newField(partial: { name: string; type: FieldType } & Partial<FieldSchema>): FieldSchema {
  return {
    id: "",
    system: false,
    hidden: false,
    presentable: false,
    required: false,
    ...partial,
  } as FieldSchema;
}

export const MULTI_VALUE_TYPES: FieldType[] = ["select", "relation", "file"];

export function fieldTypeLabel(type: FieldType): string {
  return FIELD_TYPES.find((f) => f.value === type)?.label ?? type;
}

/** `select`/`relation`/`file` fields express "more than one value" as
 * `maxSelect > 1` rather than a boolean — there is no `multiple` flag on
 * the wire. */
export function isMultiValue(field: { type: FieldType; maxSelect?: unknown }): boolean {
  return MULTI_VALUE_TYPES.includes(field.type) && Number(field.maxSelect ?? 1) > 1;
}

/** `id` and (for auth collections) `password`/`tokenKey`/`email`/etc. come
 * back from the API marked `system: true`. `created`/`updated` are *not*
 * flagged system (PocketBase doesn't mark them either) but are still part
 * of every collection's fixed scaffold rather than something someone typed
 * into this editor, so they're managed the same way here. */
function isManagedField(field: CollectionField): boolean {
  return field.system === true || (field.type === "autodate" && (field.name === "created" || field.name === "updated"));
}

/** A collection's user-defined fields, in dashboard shape. Every renderer
 * here wants only these — the same set the deleted in-house SDK used to
 * hand back as `collection.schema`, with `id`/`created`/`updated`/auth
 * system columns already filtered out. */
export function userFields(collection: CollectionModel): FieldSchema[] {
  return collection.fields.filter((f) => !isManagedField(f)) as FieldSchema[];
}

/** The complement of `userFields` — `id`, `created`, `updated`, and the
 * auth system columns. The collection editor never shows or edits these,
 * but PATCHing a collection replaces its whole `fields` array wholesale, so
 * they have to be spliced back in around the edited user fields or an
 * update would silently delete them. */
export function managedFields(collection: CollectionModel): FieldSchema[] {
  return collection.fields.filter(isManagedField) as FieldSchema[];
}

/** `created`/`updated` for a brand-new collection. The create endpoint only
 * guarantees `id` (and, for auth collections, the auth columns) on a
 * client-submitted collection — unlike the `meta/scaffolds` template it
 * doesn't add these two on its own, so a collection created without them
 * would end up with no timestamp columns at all. */
export function defaultTimestampFields(): FieldSchema[] {
  return [
    newField({ name: "created", type: "autodate", onCreate: true, onUpdate: false }),
    newField({ name: "updated", type: "autodate", onCreate: true, onUpdate: true }),
  ];
}
