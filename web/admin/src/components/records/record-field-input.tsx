import { useQuery } from "@tanstack/react-query";
import type { FieldSchema, RecordModel } from "cratebase";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { TagInput } from "@/components/interior/tag-input";
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

export function RecordFieldInput({ field, value, onChange, error }: RecordFieldInputProps) {
  const options = field.options ?? {};
  const multiple = Boolean(options.multiple);
  const relationOptions = useRelationOptions(
    field.type === "relation" ? (options.collectionId as string | undefined) : undefined,
  ).data;
  const inputClass = `h-9 w-full rounded-[9px] border-2 bg-secondary/60 px-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:bg-card ${
    error ? "border-destructive" : "border-border focus:border-primary"
  }`;

  switch (field.type) {
    case "bool":
      return <Switch checked={Boolean(value)} onCheckedChange={onChange} />;

    case "number":
      return (
        <input
          type="number"
          value={typeof value === "number" ? value : ""}
          onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))}
          className={inputClass}
        />
      );

    case "date":
      return (
        <input
          type="datetime-local"
          value={toDatetimeLocal(value)}
          onChange={(e) => onChange(e.target.value ? new Date(e.target.value).toISOString() : null)}
          className={inputClass}
        />
      );

    case "editor":
      return (
        <Textarea
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.target.value)}
          rows={6}
          className={error ? "border-destructive" : undefined}
        />
      );

    case "json":
      return (
        <Textarea
          value={typeof value === "string" ? value : JSON.stringify(value ?? null, null, 2)}
          onChange={(e) => onChange(e.target.value)}
          rows={5}
          className={`font-mono text-[12px] ${error ? "border-destructive" : ""}`}
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
        <select value={typeof value === "string" ? value : ""} onChange={(e) => onChange(e.target.value)} className={inputClass}>
          <option value="">—</option>
          {values.map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </select>
      );
    }

    case "relation": {
      if (multiple) {
        const selected = Array.isArray(value) ? (value as string[]) : [];
        return (
          <select
            multiple
            value={selected}
            onChange={(e) => onChange(Array.from(e.target.selectedOptions, (o) => o.value))}
            className={`${inputClass} h-24`}
          >
            {relationOptions?.map((opt) => (
              <option key={opt.id} value={opt.id}>
                {opt.label}
              </option>
            ))}
          </select>
        );
      }
      return (
        <select value={typeof value === "string" ? value : ""} onChange={(e) => onChange(e.target.value || null)} className={inputClass}>
          <option value="">—</option>
          {relationOptions?.map((opt) => (
            <option key={opt.id} value={opt.id}>
              {opt.label}
            </option>
          ))}
        </select>
      );
    }

    case "password":
      return <input type="password" value={typeof value === "string" ? value : ""} onChange={(e) => onChange(e.target.value)} className={inputClass} />;

    case "email":
    case "url":
    case "text":
    default:
      return (
        <input
          type={field.type === "email" ? "email" : field.type === "url" ? "url" : "text"}
          value={typeof value === "string" ? value : ""}
          onChange={(e) => onChange(e.target.value)}
          className={inputClass}
        />
      );
  }
}

export function existingRecordValue(record: RecordModel | null, field: FieldSchema): unknown {
  if (!record) return field.type === "bool" ? false : null;
  return record[field.name] ?? null;
}
