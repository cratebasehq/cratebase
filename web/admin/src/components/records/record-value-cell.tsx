import { useState } from "react";
import type { FieldSchema, RecordModel } from "cratebase";
import { FileIcon } from "lucide-react";
import { cb } from "@/lib/api";
import { CopyButton } from "@/components/interior/copy-button";
import { Lightbox } from "@/components/interior/lightbox";
import { Badge } from "@/components/ui/badge";

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|avif|svg)$/i;

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export function IdCell({ id }: { id: string }) {
  return (
    <div className="flex items-center gap-1 font-mono text-[12px] text-muted-foreground">
      <span className="truncate">{id.slice(0, 8)}</span>
      <CopyButton value={id} label="Copy id" />
    </div>
  );
}

function FileValue({ record, field, filename }: { record: RecordModel; field: FieldSchema; filename: string }) {
  const [open, setOpen] = useState(false);
  const url = cb.getFileUrl(record, filename);
  const isImage = IMAGE_EXT.test(filename);

  if (!isImage) {
    return (
      <a
        href={url}
        target="_blank"
        rel="noreferrer"
        onClick={(e) => e.stopPropagation()}
        className="flex items-center gap-1 text-[12.5px] text-primary hover:underline"
      >
        <FileIcon className="size-3.5" />
        <span className="max-w-32 truncate">{filename}</span>
      </a>
    );
  }

  return (
    <>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          setOpen(true);
        }}
        className="block h-8 w-8 overflow-hidden rounded-md border border-border"
      >
        <img src={url} alt="" className="h-full w-full object-cover" />
      </button>
      <Lightbox
        open={open}
        onClose={() => setOpen(false)}
        src={url}
        alt={filename}
        caption={field.name}
      />
    </>
  );
}

export function RecordValueCell({ record, field }: { record: RecordModel; field: FieldSchema }) {
  const value = record[field.name];

  if (value === null || value === undefined || value === "") {
    return <span className="text-muted-foreground/60">—</span>;
  }

  switch (field.type) {
    case "bool":
      return (
        <Badge variant={value ? "default" : "secondary"} className="font-normal">
          {value ? "true" : "false"}
        </Badge>
      );
    case "date":
      return <span className="font-mono text-[12px] tabular-nums">{formatDate(String(value))}</span>;
    case "number":
      return <span className="font-mono text-[12.5px] tabular-nums">{String(value)}</span>;
    case "json":
      return <code className="font-mono text-[12px] text-muted-foreground">{JSON.stringify(value)}</code>;
    case "select":
      return Array.isArray(value) ? (
        <div className="flex flex-wrap gap-1">
          {value.map((v) => (
            <Badge key={String(v)} variant="outline" className="font-normal">
              {String(v)}
            </Badge>
          ))}
        </div>
      ) : (
        <Badge variant="outline" className="font-normal">
          {String(value)}
        </Badge>
      );
    case "relation":
      return Array.isArray(value) ? (
        <span className="font-mono text-[12px] text-muted-foreground">{value.length} linked</span>
      ) : (
        <span className="font-mono text-[12px] text-muted-foreground">{String(value).slice(0, 8)}</span>
      );
    case "file": {
      const filenames = Array.isArray(value) ? (value as string[]) : [String(value)];
      return (
        <div className="flex flex-wrap gap-1.5">
          {filenames.map((filename) => (
            <FileValue key={filename} record={record} field={field} filename={filename} />
          ))}
        </div>
      );
    }
    default:
      return <span className="truncate text-[13px]">{String(value)}</span>;
  }
}
