import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { CollectionModel } from "@cratebase/client";
import { AlertCircle, FileJson, Upload } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { StatusBadge } from "@/components/layout/stat-card";
import { useCollections } from "@/hooks/use-collections";
import { cb, describeFailure } from "@/lib/api";
import { managedFields } from "@/lib/field-types";

interface ImportCollectionsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Parses whatever was pasted or dropped in. The dashboard's own export is a
 * bare array; some exports (and hand-written fixtures) wrap it as
 * `{ collections: [...] }` — accept both rather than making someone strip
 * the wrapper themselves. */
function parseCollections(raw: string): CollectionModel[] {
  const parsed = JSON.parse(raw);
  const list = Array.isArray(parsed) ? parsed : Array.isArray(parsed?.collections) ? parsed.collections : null;
  if (!list) throw new Error("Expected a JSON array of collections.");
  return list as CollectionModel[];
}

type ImportAction = "create" | "update" | "unchanged" | "delete";

interface ImportRow {
  name: string;
  type: string;
  action: ImportAction;
  detail: string;
}

const RULE_KEYS = ["listRule", "viewRule", "createRule", "updateRule", "deleteRule"] as const;

/** Dry-run of what the import would do, matched by name (then id) against
 * the live schema. Mirrors the server's create-or-update behaviour so the
 * preview is a faithful warning, not a guess. */
function previewImport(incoming: CollectionModel[], existing: CollectionModel[], deleteMissing: boolean): ImportRow[] {
  const rows: ImportRow[] = [];
  const matched = new Set<string>();
  for (const next of incoming) {
    const current = existing.find((c) => c.name === next.name) ?? existing.find((c) => next.id && c.id === next.id);
    if (!current) {
      rows.push({
        name: next.name,
        type: next.type ?? "base",
        action: "create",
        detail: `${(next.fields ?? []).length} field${(next.fields ?? []).length === 1 ? "" : "s"}`,
      });
      continue;
    }
    matched.add(current.id);
    const before = new Map(managedFields(current).map((f) => [f.name, f.type]));
    const after = new Map((next.fields ?? []).map((f) => [f.name, f.type]));
    const added = [...after.keys()].filter((n) => !before.has(n));
    const removed = [...before.keys()].filter((n) => !after.has(n));
    const retyped = [...after.keys()].filter((n) => before.has(n) && before.get(n) !== after.get(n));
    const rulesChanged = RULE_KEYS.filter((k) => (current[k] ?? null) !== (next[k] ?? null));
    const parts: string[] = [];
    if (added.length) parts.push(`+${added.length} field${added.length === 1 ? "" : "s"}`);
    if (removed.length) parts.push(`${deleteMissing ? "−" : "keeps "}${removed.length} field${removed.length === 1 ? "" : "s"}`);
    if (retyped.length) parts.push(`${retyped.length} retyped`);
    if (rulesChanged.length) parts.push(`${rulesChanged.length} rule${rulesChanged.length === 1 ? "" : "s"}`);
    const unchanged = added.length + retyped.length + rulesChanged.length === 0 && (!deleteMissing || removed.length === 0);
    rows.push({
      name: next.name,
      type: next.type ?? current.type,
      action: unchanged ? "unchanged" : "update",
      detail: parts.join(" · ") || "no differences",
    });
  }
  if (deleteMissing) {
    for (const c of existing) {
      if (!matched.has(c.id) && !c.system) {
        rows.push({ name: c.name, type: c.type, action: "delete", detail: "and every record in it" });
      }
    }
  }
  return rows;
}

const ACTION_TONE = { create: "success", update: "info", unchanged: "neutral", delete: "danger" } as const;
const ACTION_LABEL = { create: "New", update: "Update", unchanged: "No change", delete: "Delete" } as const;

/**
 * `PUT /api/collections/import`: replace or extend the schema from a JSON
 * export. The whole thing runs in one transaction server-side — a single
 * invalid collection leaves the schema exactly as it was, so this doesn't
 * need to pre-validate anything beyond "is this parseable JSON".
 */
export function ImportCollectionsDialog({ open, onOpenChange }: ImportCollectionsDialogProps) {
  const [text, setText] = useState("");
  const [deleteMissing, setDeleteMissing] = useState(false);
  const [parseError, setParseError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { data: existing = [] } = useCollections();

  /** Live parse of the textarea: either the list it holds or why it isn't one. */
  const parsed = useMemo<{ list: CollectionModel[] } | { error: string } | null>(() => {
    if (!text.trim()) return null;
    try {
      return { list: parseCollections(text) };
    } catch (error) {
      return { error: error instanceof SyntaxError ? `Invalid JSON — ${error.message}` : (error as Error).message };
    }
  }, [text]);
  const rows = useMemo(
    () => (parsed && "list" in parsed ? previewImport(parsed.list, existing, deleteMissing) : []),
    [parsed, existing, deleteMissing],
  );
  const deletions = rows.filter((r) => r.action === "delete").length;

  useEffect(() => {
    if (open) {
      setText("");
      setDeleteMissing(false);
      setParseError(null);
    }
  }, [open]);

  const importMutation = useMutation({
    mutationFn: async () => {
      const collections = parseCollections(text);
      await cb.admin.collections.import(collections, deleteMissing);
      return collections.length;
    },
    onSuccess: async (count) => {
      await queryClient.invalidateQueries({ queryKey: ["collections"] });
      toast.success(`Imported ${count} collection${count === 1 ? "" : "s"}`);
      onOpenChange(false);
      void navigate({ to: "/" });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      const fieldMessages = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
      setParseError(fieldMessages.length > 0 ? fieldMessages.join("; ") : failure.serverMessage || failure.detail);
      toast.error(failure.title);
    },
  });

  function handleFile(file: File) {
    const reader = new FileReader();
    reader.onload = () => setText(String(reader.result ?? ""));
    reader.readAsText(file);
  }

  function handleImport() {
    setParseError(null);
    try {
      const collections = parseCollections(text);
      if (collections.length === 0) {
        setParseError("The file contains no collections.");
        return;
      }
    } catch (error) {
      setParseError(error instanceof Error ? error.message : "Invalid JSON.");
      return;
    }
    importMutation.mutate();
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Import collections</DialogTitle>
          <DialogDescription>
            Paste a collections export, or load one from disk. Every collection in it is created or updated to
            match; nothing else in the schema is touched unless you enable the option below.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <Label htmlFor="import-json" className="text-xs">
              Collections JSON
            </Label>
            <input
              ref={fileInputRef}
              type="file"
              accept="application/json,.json"
              className="hidden"
              onChange={(e) => {
                const file = e.target.files?.[0];
                e.target.value = "";
                if (file) handleFile(file);
              }}
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-control-sm gap-1.5"
              onClick={() => fileInputRef.current?.click()}
            >
              <Upload className="size-3.5" />
              Load from file
            </Button>
          </div>
          <Textarea
            id="import-json"
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setParseError(null);
            }}
            placeholder='[{ "name": "posts", "type": "base", "fields": [...] }]'
            rows={12}
            className="font-mono text-xs"
            aria-invalid={parseError ? true : undefined}
          />
          {parsed && "error" in parsed ? (
            <p role="alert" className="flex items-start gap-1 text-xs font-medium text-destructive">
              <AlertCircle className="mt-0.5 size-3 shrink-0" />
              {parsed.error}
            </p>
          ) : null}
          {parseError ? (
            <p role="alert" className="flex items-start gap-1 text-xs font-medium text-destructive">
              <AlertCircle className="mt-0.5 size-3 shrink-0" />
              {parseError}
            </p>
          ) : null}

          {rows.length > 0 ? (
            <div className="flex flex-col gap-1.5">
              <span className="text-xs font-medium text-muted-foreground">
                Preview — {rows.filter((r) => r.action !== "delete").length} in file
                {deletions > 0 ? `, ${deletions} to delete` : ""}
              </span>
              <ul className="max-h-48 overflow-y-auto rounded-lg border border-border bg-card">
                {rows.map((row) => (
                  <li
                    key={`${row.action}:${row.name}`}
                    className="flex items-center gap-2 border-b border-border px-3 py-1.5 last:border-b-0"
                  >
                    <FileJson className="size-3.5 shrink-0 text-muted-foreground" />
                    <span className="font-mono text-sm">{row.name}</span>
                    <span className="text-2xs text-muted-foreground">{row.type}</span>
                    <span className="min-w-0 flex-1 truncate text-right text-xs text-muted-foreground">{row.detail}</span>
                    <StatusBadge tone={ACTION_TONE[row.action]}>{ACTION_LABEL[row.action]}</StatusBadge>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}

          <label className="flex items-start gap-2 text-sm">
            <Checkbox
              checked={deleteMissing}
              onCheckedChange={(checked) => setDeleteMissing(checked === true)}
              className="mt-0.5"
            />
            <span>
              Delete collections and fields not present in this import
              <span className="block text-xs text-muted-foreground">
                Removes collections that are not in the file. Off by default so a partial export never
                drops the rest of your schema.
              </span>
            </span>
          </label>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            variant={deletions > 0 ? "destructive" : "default"}
            onClick={handleImport}
            disabled={!parsed || "error" in parsed || importMutation.isPending}
          >
            {importMutation.isPending ? <Spinner /> : null}
            {importMutation.isPending
              ? "Importing…"
              : deletions > 0
                ? `Import and delete ${deletions}`
                : "Import"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
