import { useState } from "react";
import { AlertCircle, Code2, Pencil, Plus, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  buildIndex,
  parseIndex,
  suggestIndexName,
  validateIndex,
  type ParsedIndex,
} from "@/lib/collection-indexes";

function IndexForm({
  collectionName,
  columnOptions,
  initial,
  otherStatements,
  onCancel,
  onSubmit,
}: {
  collectionName: string;
  columnOptions: string[];
  initial: ParsedIndex;
  otherStatements: string[];
  onCancel: () => void;
  onSubmit: (statement: string) => void;
}) {
  const [draft, setDraft] = useState<ParsedIndex>(initial);
  const [renamed, setRenamed] = useState(initial.name.length > 0);
  const error = validateIndex(draft, otherStatements);

  function setColumns(next: string[]) {
    setDraft((prev) => ({
      ...prev,
      columns: next,
      name: renamed ? prev.name : suggestIndexName(collectionName, next),
    }));
  }

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border bg-surface-sunken/60 p-3">
      <div className="flex flex-wrap gap-1.5">
        {columnOptions.map((column) => {
          const on = draft.columns.includes(column);
          return (
            <button
              key={column}
              type="button"
              onClick={() => setColumns(on ? draft.columns.filter((c) => c !== column) : [...draft.columns, column])}
              className={cn(
                "rounded-md border px-2 py-1 font-mono text-2xs transition-colors",
                on
                  ? "border-primary bg-primary text-primary-foreground"
                  : "border-border text-muted-foreground hover:border-border-strong hover:text-foreground",
              )}
            >
              {column}
              {on && draft.columns.length > 1 ? (
                <span className="ml-1 font-tabular opacity-70">{draft.columns.indexOf(column) + 1}</span>
              ) : null}
            </button>
          );
        })}
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto]">
        <label className="flex flex-col gap-1">
          <span className="text-xs font-medium text-foreground/80">Index name</span>
          <Input
            value={draft.name}
            onChange={(e) => {
              setRenamed(true);
              setDraft({ ...draft, name: e.target.value });
            }}
            placeholder={suggestIndexName(collectionName, draft.columns)}
            className="h-control-md font-mono text-sm"
          />
        </label>
        <label className="flex cursor-pointer items-center gap-2 self-end pb-2">
          <Checkbox
            checked={draft.unique}
            onCheckedChange={(next) => setDraft({ ...draft, unique: next === true })}
          />
          <span className="text-xs">Unique</span>
        </label>
      </div>

      <label className="flex flex-col gap-1">
        <span className="text-xs font-medium text-foreground/80">Partial index predicate (optional)</span>
        <Input
          value={draft.where}
          onChange={(e) => setDraft({ ...draft, where: e.target.value })}
          placeholder="e.g. published = true"
          className="h-control-md font-mono text-sm"
        />
        <span className="text-2xs leading-snug text-muted-foreground">
          Raw SQL, evaluated by SQLite — a unique index with a predicate only constrains the matching rows.
        </span>
      </label>

      <pre className="overflow-x-auto rounded border border-border bg-background px-2 py-1.5 font-mono text-2xs text-muted-foreground">
        {buildIndex(collectionName, draft)}
      </pre>

      {error ? (
        <p className="flex items-center gap-1 text-xs text-destructive">
          <AlertCircle className="size-3 shrink-0" />
          {error}
        </p>
      ) : null}

      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          type="button"
          size="sm"
          disabled={error !== null}
          onClick={() => onSubmit(buildIndex(collectionName, draft))}
        >
          Save index
        </Button>
      </div>
    </div>
  );
}

export function IndexEditor({
  collectionName,
  columnOptions,
  value,
  onChange,
}: {
  collectionName: string;
  /** Every column an index may reference, in schema order. */
  columnOptions: string[];
  value: string[];
  onChange: (next: string[]) => void;
}) {
  /** `-1` means "the add form"; `null` means no form is open. */
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [rawIndex, setRawIndex] = useState<number | null>(null);
  const [rawDraft, setRawDraft] = useState("");

  function replace(at: number, statement: string) {
    const next = [...value];
    next[at] = statement;
    onChange(next);
  }

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <div className="flex flex-col">
          <span className="text-sm font-medium text-foreground">Indexes</span>
          <span className="text-xs text-muted-foreground">
            Raw <code className="font-mono">CREATE INDEX</code> statements. A unique index is the only way to
            constrain a column to distinct values.
          </span>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-control-sm shrink-0 gap-1.5"
          onClick={() => {
            setRawIndex(null);
            setEditingIndex(-1);
          }}
        >
          <Plus className="size-3.5" />
          Add index
        </Button>
      </div>

      {value.length === 0 && editingIndex === null ? (
        <p className="rounded-lg border border-dashed border-border p-4 text-center text-sm text-muted-foreground">
          No indexes. Every lookup on this collection is a full scan.
        </p>
      ) : null}

      <div className="flex flex-col gap-2">
        {value.map((statement, i) => {
          const parsed = parseIndex(statement);
          if (editingIndex === i && parsed) {
            return (
              <IndexForm
                key={i}
                collectionName={collectionName}
                columnOptions={columnOptions}
                initial={parsed}
                otherStatements={value.filter((_, j) => j !== i)}
                onCancel={() => setEditingIndex(null)}
                onSubmit={(next) => {
                  replace(i, next);
                  setEditingIndex(null);
                }}
              />
            );
          }
          if (rawIndex === i) {
            return (
              <div key={i} className="flex flex-col gap-2 rounded-lg border border-border bg-surface-sunken/60 p-3">
                <textarea
                  value={rawDraft}
                  onChange={(e) => setRawDraft(e.target.value)}
                  spellCheck={false}
                  rows={3}
                  aria-label="Index statement"
                  className="w-full resize-y rounded-md border border-input bg-background p-2 font-mono text-sm outline-none focus-visible:border-ring"
                />
                <div className="flex justify-end gap-2">
                  <Button type="button" variant="ghost" size="sm" onClick={() => setRawIndex(null)}>
                    Cancel
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    disabled={rawDraft.trim().length === 0}
                    onClick={() => {
                      replace(i, rawDraft.trim());
                      setRawIndex(null);
                    }}
                  >
                    Save statement
                  </Button>
                </div>
              </div>
            );
          }
          return (
            <div
              key={i}
              className="group flex items-start gap-2 rounded-lg border border-border bg-card px-3 py-2"
            >
              <Code2 className="mt-1 size-3.5 shrink-0 text-muted-foreground/60" />
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="font-mono text-sm">{parsed?.name ?? "custom statement"}</span>
                  {parsed?.unique ? (
                    <Badge variant="outline" className="font-normal">
                      unique
                    </Badge>
                  ) : null}
                  {parsed?.where ? (
                    <Badge variant="outline" className="font-normal">
                      partial
                    </Badge>
                  ) : null}
                  {parsed ? (
                    <span className="font-mono text-2xs text-muted-foreground">({parsed.columns.join(", ")})</span>
                  ) : null}
                </div>
                <code className="truncate font-mono text-2xs text-muted-foreground/70" title={statement}>
                  {statement}
                </code>
              </div>
              <div className="flex shrink-0 items-center gap-0.5">
                <button
                  type="button"
                  aria-label={parsed ? "Edit index" : "Edit statement"}
                  onClick={() => {
                    if (parsed) {
                      setRawIndex(null);
                      setEditingIndex(i);
                    } else {
                      setEditingIndex(null);
                      setRawDraft(statement);
                      setRawIndex(i);
                    }
                  }}
                  className="grid size-control-xs place-items-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                >
                  <Pencil className="size-3.5" />
                </button>
                <button
                  type="button"
                  aria-label="Edit as SQL"
                  title="Edit as SQL"
                  onClick={() => {
                    setEditingIndex(null);
                    setRawDraft(statement);
                    setRawIndex(i);
                  }}
                  className="grid size-control-xs place-items-center rounded font-mono text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                >
                  SQL
                </button>
                <button
                  type="button"
                  aria-label="Remove index"
                  onClick={() => onChange(value.filter((_, j) => j !== i))}
                  className="grid size-control-xs place-items-center rounded text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                >
                  <Trash2 className="size-3.5" />
                </button>
              </div>
            </div>
          );
        })}

        {editingIndex === -1 ? (
          <IndexForm
            collectionName={collectionName}
            columnOptions={columnOptions}
            initial={{ name: "", unique: false, columns: [], where: "" }}
            otherStatements={value}
            onCancel={() => setEditingIndex(null)}
            onSubmit={(statement) => {
              onChange([...value, statement]);
              setEditingIndex(null);
            }}
          />
        ) : null}
      </div>
    </section>
  );
}
