import type { FieldType } from "cratebase";

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
];

export const MULTI_VALUE_TYPES: FieldType[] = ["select", "relation", "file"];

export function fieldTypeLabel(type: FieldType): string {
  return FIELD_TYPES.find((f) => f.value === type)?.label ?? type;
}
