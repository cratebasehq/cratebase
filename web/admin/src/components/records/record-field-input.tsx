import { useState } from "react";
import { Braces, WrapText } from "lucide-react";
import { cn } from "@/lib/utils";
import { type FieldSchema, isMultiValue } from "@/lib/field-types";
import { Input } from "@/components/ui/input";
import { RelationPicker } from "@/components/records/relation-picker";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { TagInput } from "@/components/ui/tag-input";

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

/**
 * JSON held as text, so a half-typed document survives a re-render, with
 * the parse result reported as you type and a one-click reformat. The old
 * control was a bare textarea that handed the server a string whenever it
 * was touched and the original object whenever it wasn't.
 */
function JsonInput({
  value,
  onChange,
  invalid,
  label,
}: {
  value: unknown;
  onChange: (value: unknown) => void;
  invalid?: boolean;
  label: string;
}) {
  const text = typeof value === "string" ? value : JSON.stringify(value ?? null, null, 2);
  const [parseError, setParseError] = useState<string | null>(null);

  function format() {
    try {
      onChange(JSON.stringify(JSON.parse(text), null, 2));
      setParseError(null);
    } catch (error) {
      setParseError(error instanceof Error ? error.message : "Not valid JSON");
    }
  }

  return (
    <div className="flex flex-col gap-1">
      <Textarea
        value={text}
        aria-label={label}
        spellCheck={false}
        onChange={(e) => {
          onChange(e.target.value);
          try {
            JSON.parse(e.target.value);
            setParseError(null);
          } catch (error) {
            setParseError(error instanceof Error ? error.message : "Not valid JSON");
          }
        }}
        rows={6}
        aria-invalid={invalid || (parseError ? true : undefined)}
        className={cn("font-mono text-sm", parseError && "border-destructive")}
      />
      <div className="flex items-center justify-between gap-2">
        <span className={cn("min-w-0 truncate text-2xs", parseError ? "text-destructive" : "text-muted-foreground")}>
          {parseError ?? (
            <span className="inline-flex items-center gap-1">
              <Braces className="size-3" />
              Valid JSON
            </span>
          )}
        </span>
        <button
          type="button"
          onClick={format}
          className="inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <WrapText className="size-3" />
          Format
        </button>
      </div>
    </div>
  );
}

export function RecordFieldInput({ field, value, onChange, error }: RecordFieldInputProps) {
  const multiple = isMultiValue(field);
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
      return <JsonInput value={value} onChange={onChange} invalid={invalid} label={field.name} />;

    case "select": {
      const values = (field.values as string[] | undefined) ?? [];
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
      const ids = Array.isArray(value) ? (value as string[]) : value ? [String(value)] : [];
      return (
        <RelationPicker
          collectionId={field.collectionId as string | undefined}
          fieldName={field.name}
          value={ids}
          multiple={multiple}
          maxSelect={multiple ? Number(field.maxSelect ?? 0) : 1}
          invalid={invalid}
          onChange={(next) => onChange(multiple ? next : (next[0] ?? null))}
        />
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
