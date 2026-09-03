import { useEffect, useRef, useState } from "react";
import type { FieldSchema, RecordModel } from "cratebase";
import { toast } from "sonner";
import { useRecordMutations } from "@/hooks/use-records";
import { RecordValueCell } from "./record-value-cell";

const INLINE_SCALAR_TYPES = new Set(["text", "email", "url", "number", "date"]);

/** Fields whose value can be edited directly in the table cell, Drizzle-style,
 * without opening the full record drawer. Multi-value and structured types
 * (json, relation, file, editor, password, multi-select) still need the drawer. */
export function isInlineEditable(field: FieldSchema): boolean {
  if (field.type === "bool") return true;
  if (field.type === "select") return !field.options?.multiple;
  return INLINE_SCALAR_TYPES.has(field.type);
}

function toInputValue(field: FieldSchema, value: unknown): string {
  if (value === null || value === undefined) return "";
  if (field.type === "date") {
    const date = new Date(String(value));
    if (Number.isNaN(date.getTime())) return "";
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
  }
  return String(value);
}

function fromInputValue(field: FieldSchema, raw: string): unknown {
  if (raw === "") return field.required ? "" : null;
  if (field.type === "number") return Number(raw);
  if (field.type === "date") {
    const date = new Date(raw);
    return Number.isNaN(date.getTime()) ? null : date.toISOString();
  }
  return raw;
}

/** A table cell that behaves like a spreadsheet/Drizzle-Studio cell: click to
 * edit in place, Enter or blur to save, Escape to cancel. Falls back to
 * opening the full record drawer for types that need more than one input. */
export function InlineEditableCell({
  record,
  field,
  collectionName,
  onOpenDrawer,
}: {
  record: RecordModel;
  field: FieldSchema;
  collectionName: string;
  onOpenDrawer: () => void;
}) {
  const { update } = useRecordMutations(collectionName);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement | HTMLSelectElement>(null);

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  if (!isInlineEditable(field)) {
    return (
      <div
        role="button"
        tabIndex={0}
        onClick={onOpenDrawer}
        onKeyDown={(e) => e.key === "Enter" && onOpenDrawer()}
        className="block w-full cursor-pointer text-left"
      >
        <RecordValueCell record={record} field={field} />
      </div>
    );
  }

  async function save(next: unknown) {
    const current = record[field.name];
    const unchanged = next === current || ((next === null || next === "") && (current === null || current === undefined || current === ""));
    if (unchanged) return;
    try {
      await update.mutateAsync({ id: record.id, data: { [field.name]: next } });
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to update value");
    }
  }

  if (field.type === "bool") {
    return (
      <button
        type="button"
        disabled={update.isPending}
        onClick={() => void save(!record[field.name])}
        className="block text-left disabled:opacity-50"
      >
        <RecordValueCell record={record} field={field} />
      </button>
    );
  }

  function startEditing() {
    setDraft(toInputValue(field, record[field.name]));
    setEditing(true);
  }

  function commit(rawOverride?: string) {
    setEditing(false);
    void save(fromInputValue(field, rawOverride ?? draft));
  }

  if (!editing) {
    return (
      <div
        role="button"
        tabIndex={0}
        onDoubleClick={startEditing}
        onKeyDown={(e) => e.key === "Enter" && startEditing()}
        title="Double-click to edit"
        className="-mx-1.5 -my-0.5 block w-full cursor-default rounded px-1.5 py-0.5 text-left outline-none transition-colors hover:bg-accent/60 focus-visible:ring-1 focus-visible:ring-primary"
      >
        <RecordValueCell record={record} field={field} />
      </div>
    );
  }

  if (field.type === "select") {
    const values = (field.options?.values as string[] | undefined) ?? [];
    return (
      <select
        ref={inputRef as React.RefObject<HTMLSelectElement>}
        defaultValue={String(record[field.name] ?? "")}
        onChange={(e) => commit(e.target.value)}
        onBlur={() => setEditing(false)}
        onKeyDown={(e) => e.key === "Escape" && setEditing(false)}
        className="h-7 w-full rounded border border-primary bg-background px-1.5 text-[13px] outline-none"
      >
        {!field.required && <option value="">—</option>}
        {values.map((v) => (
          <option key={v} value={v}>
            {v}
          </option>
        ))}
      </select>
    );
  }

  return (
    <input
      ref={inputRef as React.RefObject<HTMLInputElement>}
      type={field.type === "number" ? "number" : field.type === "date" ? "datetime-local" : field.type}
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => commit()}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        }
        if (e.key === "Escape") {
          e.stopPropagation();
          setEditing(false);
        }
      }}
      className="h-7 w-full rounded border border-primary bg-background px-1.5 font-mono text-[12.5px] outline-none"
    />
  );
}
