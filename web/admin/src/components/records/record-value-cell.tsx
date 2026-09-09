import { useMemo, useState } from "react";
import type { RecordModel, CollectionModel } from "@cratebase/client";
import { type FieldSchema, userFields } from "@/lib/field-types";
import { useNavigate } from "@tanstack/react-router";
import {
  Check,
  X,
  ChevronDown,
  ExternalLink,
  FileIcon,
  FileText,
  FileArchive,
  FileAudio,
  FileVideo,
  Copy,
} from "lucide-react";
import { cb } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Lightbox } from "@/components/ui/lightbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Badge } from "@/components/ui/badge";

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|avif|svg)$/i;
const DOC_EXT = /\.pdf$/i;
const ARCHIVE_EXT = /\.(zip|tar|gz|tgz|rar|7z)$/i;
const AUDIO_EXT = /\.(mp3|wav|ogg|flac|m4a)$/i;
const VIDEO_EXT = /\.(mp4|mov|webm|mkv|avi)$/i;

/** Chip colors cycle through a fixed, hash-stable palette so the same select
 * option always renders the same color across rows without needing the
 * schema to declare one. */
const CHIP_PALETTE = [
  "bg-blue-500/15 text-blue-700 dark:text-blue-300",
  "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  "bg-violet-500/15 text-violet-700 dark:text-violet-300",
  "bg-rose-500/15 text-rose-700 dark:text-rose-300",
  "bg-cyan-500/15 text-cyan-700 dark:text-cyan-300",
  "bg-orange-500/15 text-orange-700 dark:text-orange-300",
  "bg-fuchsia-500/15 text-fuchsia-700 dark:text-fuchsia-300",
];

function chipColor(value: string): string {
  let hash = 0;
  for (let i = 0; i < value.length; i++) hash = (hash * 31 + value.charCodeAt(i)) | 0;
  return CHIP_PALETTE[Math.abs(hash) % CHIP_PALETTE.length] ?? "";
}

function fileIconFor(filename: string) {
  if (DOC_EXT.test(filename)) return FileText;
  if (ARCHIVE_EXT.test(filename)) return FileArchive;
  if (AUDIO_EXT.test(filename)) return FileAudio;
  if (VIDEO_EXT.test(filename)) return FileVideo;
  return FileIcon;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function formatDateTitle(value: string): string | undefined {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return undefined;
  return `${date.toLocaleString(undefined, { dateStyle: "full", timeStyle: "medium" })} · ${date.toISOString()}`;
}

/** The target collection's name, user fields and "what to show instead of
 * an id" field. Read straight out of the collections list every screen
 * already holds — no request of its own. */
function useRelationTarget(collectionId?: string) {
  const { data: collections } = useCollections();
  return useMemo(() => {
    const target = collections?.find((c: CollectionModel) => c.id === collectionId);
    if (!target) return undefined;
    const fields = userFields(target);
    return {
      collectionName: target.name,
      fields,
      displayField: fields.find((f) => f.type === "text")?.name,
    };
  }, [collections, collectionId]);
}

/** The records the server already sent back under `expand`, keyed by id.
 * The grid asks for `?expand=<relation fields>`, so a relation cell has its
 * referenced record in hand from the same request that fetched the row —
 * no second query per relation, and no 200-record prefetch to guess from. */
function expandedById(record: RecordModel, fieldName: string): Map<string, RecordModel> {
  const expanded = (record.expand as Record<string, RecordModel | RecordModel[]> | undefined)?.[fieldName];
  const out = new Map<string, RecordModel>();
  if (Array.isArray(expanded)) for (const item of expanded) out.set(item.id, item);
  else if (expanded) out.set(expanded.id, expanded);
  return out;
}

/** One relation badge: click it to preview the referenced record (its
 * first few fields, resolved from the same cache `useRelationLabels`
 * already fetched) with a link to jump straight to it and pop its own
 * drawer open — a lightweight, in-place version of "click a foreign key
 * to see what it points to" (à la Drizzle Studio's relation references),
 * instead of dumping the caller into an unlabeled 8-char id. */
function RelationRefValue({
  id,
  label,
  collectionName,
  fields,
  record,
}: {
  id: string;
  label: string | undefined;
  collectionName: string | undefined;
  fields: FieldSchema[] | undefined;
  record: RecordModel | undefined;
}) {
  const navigate = useNavigate();
  const previewFields = (fields ?? [])
    .filter((f) => !["relation", "file", "json", "editor", "password"].includes(f.type))
    .slice(0, 4);

  return (
    <Popover>
      <PopoverTrigger asChild aria-label={`Preview referenced record ${id}`}>
        <Badge
          variant="outline"
          className="max-w-32 cursor-pointer font-normal transition-colors hover:border-primary hover:bg-primary/5"
          title={id}
        >
          <span className="truncate">{label ?? id}</span>
        </Badge>
      </PopoverTrigger>
      <PopoverContent side="bottom" align="start" className="w-72 gap-0 p-0">
        <div className="flex flex-col gap-2 p-3">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {collectionName ?? "Related record"}
            </span>
            <span className="truncate font-mono text-2xs text-muted-foreground/70">{id}</span>
          </div>
          {record ? (
            <dl className="flex flex-col gap-1.5 border-t border-border pt-2">
              {previewFields.map((f) => (
                <div key={f.name} className="flex items-center justify-between gap-3 text-sm">
                  <dt className="shrink-0 text-muted-foreground">{f.name}</dt>
                  <dd className="truncate text-right text-foreground">{String(record[f.name] ?? "—")}</dd>
                </div>
              ))}
              {previewFields.length === 0 && (
                <p className="text-sm text-muted-foreground">No previewable fields on this collection.</p>
              )}
            </dl>
          ) : (
            <p className="border-t border-border pt-2 text-sm text-muted-foreground">
              This query didn't expand the relation — open the record directly.
            </p>
          )}
          {collectionName ? (
            <button
              type="button"
              onClick={() => {
                void navigate({
                  to: "/collections/$name",
                  params: { name: collectionName },
                  search: { openId: id },
                });
              }}
              className="mt-1 flex items-center justify-center gap-1.5 rounded-md border border-border bg-secondary/60 py-1.5 text-sm font-medium text-foreground transition-colors hover:bg-secondary"
            >
              Open record
              <ExternalLink className="size-3" />
            </button>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function IdCell({ id }: { id: string }) {
  const { copy, status } = useCopyToClipboard();
  return (
    <div className="group/id flex items-center gap-1.5 whitespace-nowrap font-mono text-sm text-muted-foreground">
      <span>{id}</span>
      <button
        type="button"
        aria-label="Copy id"
        title={status === "copied" ? "Copied" : "Copy id"}
        onClick={(e) => {
          e.stopPropagation();
          void copy(id);
        }}
        className="shrink-0 rounded p-0.5 opacity-0 outline-none transition-opacity hover:bg-accent focus-visible:opacity-100 focus-visible:ring-1 focus-visible:ring-primary group-hover/id:opacity-100"
      >
        {status === "copied" ? <Check className="size-3" /> : <Copy className="size-3" />}
      </button>
    </div>
  );
}

function FileValue({ record, field, filename }: { record: RecordModel; field: FieldSchema; filename: string }) {
  const [open, setOpen] = useState(false);
  const url = cb.files.url(record, filename);
  const isImage = IMAGE_EXT.test(filename);

  if (!isImage) {
    const Icon = fileIconFor(filename);
    return (
      <a
        href={url}
        target="_blank"
        rel="noreferrer"
        onClick={(e) => e.stopPropagation()}
        title={filename}
        className="flex items-center gap-1 text-sm text-primary hover:underline"
      >
        <Icon className="size-3.5 shrink-0" />
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
  const relationCollectionId =
    field.type === "relation" ? (field.collectionId as string | undefined) : undefined;
  const relationInfo = useRelationTarget(relationCollectionId);

  if (value === null || value === undefined || value === "") {
    return <span className="text-muted-foreground/60">—</span>;
  }

  switch (field.type) {
    // A boolean is not a status: filling it with a solid badge makes every
    // `true` in a 500-row grid shout. A glyph plus the literal, weighted
    // only by contrast, scans far faster down a column.
    case "bool":
      return (
        <span
          className={`inline-flex items-center gap-1 font-mono text-sm ${
            value ? "text-foreground" : "text-muted-foreground/70"
          }`}
        >
          {value ? <Check className="size-3.5" /> : <X className="size-3.5" />}
          {value ? "true" : "false"}
        </span>
      );
    case "date":
      return (
        <span className="font-mono text-sm tabular-nums" title={formatDateTitle(String(value))}>
          {formatDate(String(value))}
        </span>
      );
    case "number":
      return <span className="font-mono text-sm tabular-nums">{String(value)}</span>;
    case "json": {
      const inline = JSON.stringify(value);
      const pretty = JSON.stringify(value, null, 2);
      const truncated = inline.length > 48;
      return (
        <div onClick={(e) => e.stopPropagation()}>
          <Popover>
            <PopoverTrigger
              aria-label={`${field.name} JSON value`}
              className="flex items-center gap-1 font-mono text-sm text-muted-foreground transition-colors hover:text-foreground"
            >
              <code className="max-w-56 truncate">{truncated ? `${inline.slice(0, 48)}…` : inline}</code>
              {truncated && <ChevronDown className="size-3 shrink-0" />}
            </PopoverTrigger>
            <PopoverContent side="bottom" align="start" className="w-auto max-w-[420px] gap-0 p-0">
              <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-all p-3 font-mono text-sm text-foreground">
                {pretty}
              </pre>
            </PopoverContent>
          </Popover>
        </div>
      );
    }
    case "select":
      return Array.isArray(value) ? (
        <div className="flex flex-wrap gap-1">
          {value.map((v) => (
            <Badge key={String(v)} variant="outline" className={`font-normal ${chipColor(String(v))}`}>
              {String(v)}
            </Badge>
          ))}
        </div>
      ) : (
        <Badge variant="outline" className={`font-normal ${chipColor(String(value))}`}>
          {String(value)}
        </Badge>
      );
    case "relation": {
      const ids = Array.isArray(value) ? (value as string[]) : [String(value)];
      const shown = ids.slice(0, 3);
      const expanded = expandedById(record, field.name);
      const display = relationInfo?.displayField;
      return (
        <div className="flex flex-wrap items-center gap-1" onClick={(e) => e.stopPropagation()}>
          {shown.map((id) => {
            const target = expanded.get(id);
            return (
              <RelationRefValue
                key={id}
                id={id}
                label={target && display ? String(target[display] ?? id) : undefined}
                collectionName={relationInfo?.collectionName}
                fields={relationInfo?.fields}
                record={target}
              />
            );
          })}
          {ids.length > shown.length && (
            <span className="font-mono text-xs text-muted-foreground">+{ids.length - shown.length}</span>
          )}
        </div>
      );
    }
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
    case "email":
      return (
        <a
          href={`mailto:${value}`}
          onClick={(e) => e.stopPropagation()}
          className="block truncate text-sm text-primary hover:underline"
        >
          {String(value)}
        </a>
      );
    case "url":
      return (
        <a
          href={String(value)}
          target="_blank"
          rel="noreferrer"
          onClick={(e) => e.stopPropagation()}
          className="flex min-w-0 items-center gap-1 text-sm text-primary hover:underline"
        >
          <span className="truncate">{String(value)}</span>
          <ExternalLink className="size-3 shrink-0" />
        </a>
      );
    case "geoPoint": {
      const point = value && typeof value === "object" ? (value as { lon?: number; lat?: number }) : {};
      if (typeof point.lat !== "number" || typeof point.lon !== "number") {
        return <span className="text-muted-foreground/60">—</span>;
      }
      return (
        <span className="font-mono text-sm tabular-nums" title={`${point.lat}, ${point.lon}`}>
          {point.lat.toFixed(4)}, {point.lon.toFixed(4)}
        </span>
      );
    }
    default:
      return <span className="truncate text-sm">{String(value)}</span>;
  }
}
