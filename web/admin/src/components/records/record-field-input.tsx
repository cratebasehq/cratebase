import { useId } from "react";
import { useQuery } from "@tanstack/react-query";
import type { FieldSchema, RecordModel } from "cratebase";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { TagInput } from "@/components/ui/tag-input";
import { cb } from "@/lib/api";

interface RecordFieldInputProps {
  field: FieldSchema;
  value: unknown;
  onChange: (value: unknown) => void;
  error?: string;
}

function toDatetimeLocal(value: unknown): string {
  if (typeof value !== "string" || !value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function useRelationOptions(collectionId?: string) {
  return useQuery({
    queryKey: ["relation-options", collectionId],
    queryFn: async () => {
      const target = await cb.collections.getOne(collectionId!);
      const displayField = target.schema.find((f) => f.type === "text")?.name;
      const list = await cb.collection(collectionId!).getList(1, 100);
      return list.items.map((item) => ({
        id: item.id,
        label: displayField ? String(item[displayField] ?? item.id) : item.id,
      }));
    },
    enabled: Boolean(collectionId),
  });
}

/** Multi-value relations need many ids at once, which a Radix select can't
 * express — a checklist of the same options does, without falling back to
 * a native `<select multiple>` nobody can operate comfortably. */
function RelationChecklist({
  options,
  selected,
  onChange,
}: {
  options: { id: string; label: string }[];
  selected: string[];
  onChange: (next: string[]) => void;
}) {
  const groupId = useId();
  return (
    <div
      role="group"
      aria-label="Related records"
      className="flex max-h-40 flex-col gap-1.5 overflow-y-auto rounded-lg border border-input p-2"
    >
      {options.length === 0 ? (
        <p className="text-xs text-muted-foreground">No records to relate to yet.</p>
      ) : (
        options.map((option) => {
          const id = `${groupId}-${option.id}`;
          const checked = selected.includes(option.id);
          return (
            <div key={option.id} className="flex items-center gap-2">
              <Checkbox
                id={id}
                checked={checked}
                onCheckedChange={(next) =>
                  onChange(next === true ? [...selected, option.id] : selected.filter((v) => v !== option.id))
                }
              />
              <label htmlFor={id} className="truncate text-sm text-foreground">
                {option.label}
              </label>
            </div>
          );
        })
      )}
    </div>
  );
}

export function RecordFieldInput({ field, value, onChange, error }: RecordFieldInputProps) {
  const options = field.options ?? {};
  const multiple = Boolean(options.multiple);
  const relationOptions = useRelationOptions(
    field.type === "relation" ? (options.collectionId as string | undefined) : undefined,
  ).data;
  const invalid = error ? true : undefined;

  switch (field.type) {
    case "bool":
      return <Switch checked={Boolean(value)} onCheckedChange={onChange} />;

    case "number":
      return (
        <Input
          type="number"
          value={typeof value === "number" ? value : ""}
          onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))}
          aria-invalid={invalid}
          className="h-control-md"
        />
      );

    case "date":
      return (
        <Input
          type="datetime-local"
          value={toDatetimeLocal(value)}
          onChange={(e) => onChange(e.target.value ? new Date(e.target.value).toISOString() : null)}
          aria-invalid={invalid}
          className="h-control-md"
        />
      );

    case "autodate":
      return (
        <Input
          type="text"
          value={typeof value === "string" && value ? new Date(value).toLocaleString() : "Set automatically"}
          disabled
          readOnly
          title="Autodate fields are set by the server and can't be edited here"
          className="h-control-md cursor-not-allowed text-muted-foreground"
        />
      );

    case "editor":
      return (
        <Textarea
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.target.value)}
          rows={6}
          aria-invalid={invalid}
        />
      );

    case "json":
      return (
        <Textarea
          value={typeof value === "string" ? value : JSON.stringify(value ?? null, null, 2)}
          onChange={(e) => onChange(e.target.value)}
          rows={5}
          aria-invalid={invalid}
          className="font-mono text-sm"
        />
      );

    case "select": {
      const values = (options.values as string[] | undefined) ?? [];
      if (multiple) {
        return (
          <TagInput
            label=""
            value={Array.isArray(value) ? (value as string[]) : []}
            onChange={onChange}
            validate={(candidate) => values.includes(candidate)}
            placeholder="Type an option and press Enter"
          />
        );
      }
      return (
        <Select value={typeof value === "string" ? value : ""} onValueChange={(next) => onChange(next)}>
          <SelectTrigger aria-label={field.name} aria-invalid={invalid} className="h-control-md w-full">
            <SelectValue placeholder="—" />
          </SelectTrigger>
          <SelectContent>
            {values.map((v) => (
              <SelectItem key={v} value={v}>
                {v}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }

    case "relation": {
      if (multiple) {
        return (
          <RelationChecklist
            options={relationOptions ?? []}
            selected={Array.isArray(value) ? (value as string[]) : []}
            onChange={onChange}
          />
        );
      }
      return (
        <Select
          value={typeof value === "string" ? value : ""}
          onValueChange={(next) => onChange(next || null)}
        >
          <SelectTrigger aria-label={field.name} aria-invalid={invalid} className="h-control-md w-full">
            <SelectValue placeholder="—" />
          </SelectTrigger>
          <SelectContent>
            {(relationOptions ?? []).map((opt) => (
              <SelectItem key={opt.id} value={opt.id}>
                {opt.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }

    case "password":
      return (
        <Input
          type="password"
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.target.value)}
          aria-invalid={invalid}
          className="h-control-md"
        />
      );

    case "email":
    case "url":
    case "text":
    default:
      return (
        <Input
          type={field.type === "email" ? "email" : field.type === "url" ? "url" : "text"}
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.target.value)}
          aria-invalid={invalid}
          className="h-control-md"
        />
      );
  }
}

export function existingRecordValue(record: RecordModel | null, field: FieldSchema): unknown {
  if (!record) return field.type === "bool" ? false : null;
  return record[field.name] ?? null;
}
