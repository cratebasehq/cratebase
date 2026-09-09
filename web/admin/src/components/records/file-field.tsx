import { useRef, useState } from "react";
import { FileIcon, ImageIcon, Plus, RotateCcw, Upload, X } from "lucide-react";
import type { RecordModel } from "@cratebase/client";
import { cb } from "@/lib/api";
import { cn } from "@/lib/utils";
import { isMultiValue, type FieldSchema } from "@/lib/field-types";
import { formatBytes, type FileDraft } from "@/lib/record-validation";
import { Lightbox } from "@/components/ui/lightbox";

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|avif|svg)$/i;

/** A thumbnail for an image already stored on the record, or a type glyph
 * for anything else. */
function StoredFile({
  record,
  filename,
  onRemove,
}: {
  record: RecordModel;
  filename: string;
  onRemove: () => void;
}) {
  const [preview, setPreview] = useState(false);
  const url = cb.files.url(record, filename);
  const isImage = IMAGE_EXT.test(filename);

  return (
    <li className="group flex items-center gap-2 rounded-md border border-border bg-card px-2 py-1.5">
      {isImage ? (
        <button
          type="button"
          onClick={() => setPreview(true)}
          aria-label={`Preview ${filename}`}
          className="size-8 shrink-0 overflow-hidden rounded border border-border"
        >
          <img src={url} alt="" className="size-full object-cover" />
        </button>
      ) : (
        <span className="grid size-8 shrink-0 place-items-center rounded border border-border text-muted-foreground">
          <FileIcon className="size-4" />
        </span>
      )}
      <a
        href={url}
        target="_blank"
        rel="noreferrer"
        className="min-w-0 flex-1 truncate font-mono text-xs text-foreground hover:underline"
        title={filename}
      >
        {filename}
      </a>
      <button
        type="button"
        onClick={onRemove}
        aria-label={`Remove ${filename}`}
        className="grid size-control-xs shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
      >
        <X className="size-3.5" />
      </button>
      {isImage ? (
        <Lightbox open={preview} onClose={() => setPreview(false)} src={url} alt={filename} caption={filename} />
      ) : null}
    </li>
  );
}

/** A file picked in this session, not yet uploaded. */
function StagedFile({ file, onRemove }: { file: File; onRemove: () => void }) {
  const isImage = file.type.startsWith("image/");
  const url = isImage ? URL.createObjectURL(file) : null;
  return (
    <li className="flex items-center gap-2 rounded-md border border-primary/40 bg-primary/[0.04] px-2 py-1.5">
      {url ? (
        <img src={url} alt="" className="size-8 shrink-0 rounded border border-border object-cover" />
      ) : (
        <span className="grid size-8 shrink-0 place-items-center rounded border border-border text-muted-foreground">
          <ImageIcon className="size-4" />
        </span>
      )}
      <span className="min-w-0 flex-1 truncate font-mono text-xs" title={file.name}>
        {file.name}
      </span>
      <span className="shrink-0 font-tabular text-2xs text-muted-foreground">{formatBytes(file.size)}</span>
      <button
        type="button"
        onClick={onRemove}
        aria-label={`Remove ${file.name}`}
        className="grid size-control-xs shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <X className="size-3.5" />
      </button>
    </li>
  );
}

/**
 * The file editor the audit's "file input bare" refers to.
 *
 * The old control was a raw `<input type=file>` that appended to whatever
 * was already stored while its help text claimed it replaced it. This shows
 * what the record actually holds, lets individual files be removed (the
 * server takes `field-` by filename), stages new ones with their size, and
 * refuses a file the field's own `maxSize`/`mimeTypes`/`maxSelect` would
 * make the server reject.
 */
export function FileField({
  field,
  record,
  value,
  onChange,
  invalid,
}: {
  field: FieldSchema;
  /** The saved record, for resolving stored file URLs. `null` when new. */
  record: RecordModel | null;
  value: FileDraft;
  onChange: (next: FileDraft) => void;
  invalid?: boolean;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const multiple = isMultiValue(field);
  const maxSelect = multiple ? Number(field.maxSelect ?? 99) : 1;
  const mimeTypes = (field.mimeTypes as string[] | undefined) ?? [];
  const maxSize = Number(field.maxSize ?? 0);

  const stored = (record?.[field.name] as string[] | string | undefined) ?? [];
  const storedNames = Array.isArray(stored) ? stored : stored ? [stored] : [];
  const removed = storedNames.filter((name) => !value.keep.includes(name));
  const total = value.keep.length + value.added.length;
  const full = total >= maxSelect;

  function addFiles(files: File[]) {
    if (files.length === 0) return;
    // A single-file field replaces rather than accumulates — that is what
    // picking a file on a one-file field visibly means.
    const next = multiple ? [...value.added, ...files].slice(0, Math.max(0, maxSelect - value.keep.length)) : files.slice(0, 1);
    onChange({ keep: multiple ? value.keep : [], added: next });
  }

  return (
    <div className="flex flex-col gap-2">
      {value.keep.length > 0 || value.added.length > 0 ? (
        <ul className="flex flex-col gap-1.5">
          {record
            ? value.keep.map((name) => (
                <StoredFile
                  key={name}
                  record={record}
                  filename={name}
                  onRemove={() => onChange({ ...value, keep: value.keep.filter((n) => n !== name) })}
                />
              ))
            : null}
          {value.added.map((file, i) => (
            <StagedFile
              key={`${file.name}-${i}`}
              file={file}
              onRemove={() => onChange({ ...value, added: value.added.filter((_, j) => j !== i) })}
            />
          ))}
        </ul>
      ) : null}

      {removed.length > 0 ? (
        <p className="flex items-center gap-1.5 text-2xs text-warning">
          {removed.length} {removed.length === 1 ? "file" : "files"} will be deleted on save
          <button
            type="button"
            onClick={() => onChange({ ...value, keep: storedNames })}
            className="inline-flex items-center gap-1 rounded px-1 py-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <RotateCcw className="size-3" />
            undo
          </button>
        </p>
      ) : null}

      {!full ? (
        <div
          onDragOver={(e) => {
            e.preventDefault();
            setDragging(true);
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={(e) => {
            e.preventDefault();
            setDragging(false);
            addFiles(Array.from(e.dataTransfer.files));
          }}
          className={cn(
            "flex flex-col items-center gap-1 rounded-lg border border-dashed px-3 py-4 text-center transition-colors",
            dragging ? "border-primary bg-primary/[0.06]" : "border-border",
            invalid && "border-destructive/60",
          )}
        >
          <Upload className="size-4 text-muted-foreground" />
          <button
            type="button"
            onClick={() => inputRef.current?.click()}
            className="inline-flex items-center gap-1 text-sm font-medium text-foreground hover:underline"
          >
            <Plus className="size-3.5" />
            Choose {multiple ? "files" : "a file"}
          </button>
          <span className="text-2xs leading-snug text-muted-foreground">
            or drop {multiple ? "them" : "it"} here
            {mimeTypes.length > 0 ? ` · ${mimeTypes.join(", ")}` : ""}
            {maxSize > 0 ? ` · up to ${formatBytes(maxSize)}` : ""}
            {multiple ? ` · ${total}/${maxSelect}` : ""}
          </span>
          <input
            ref={inputRef}
            type="file"
            hidden
            aria-label={field.name}
            multiple={multiple}
            accept={mimeTypes.length > 0 ? mimeTypes.join(",") : undefined}
            onChange={(e) => {
              addFiles(Array.from(e.target.files ?? []));
              // Let the same file be picked again after being removed.
              e.target.value = "";
            }}
          />
        </div>
      ) : (
        <p className="text-2xs text-muted-foreground">
          {multiple ? `${total} of ${maxSelect} files.` : "One file."} Remove one to pick another.
        </p>
      )}
    </div>
  );
}
