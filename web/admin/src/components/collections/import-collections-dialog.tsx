import { useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { CollectionModel } from "@cratebase/client";
import { AlertCircle, Upload } from "lucide-react";
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
import { cb } from "@/lib/api";

interface ImportCollectionsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Parses whatever was pasted or dropped in. PocketBase's own export is a
 * bare array; some exports (and hand-written fixtures) wrap it as
 * `{ collections: [...] }` — accept both rather than making someone strip
 * the wrapper themselves. */
function parseCollections(raw: string): CollectionModel[] {
  const parsed = JSON.parse(raw);
  const list = Array.isArray(parsed) ? parsed : Array.isArray(parsed?.collections) ? parsed.collections : null;
  if (!list) throw new Error("Expected a JSON array of collections.");
  return list as CollectionModel[];
}

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
      setParseError(null);
      toast.error(error instanceof Error ? error.message : "Failed to import collections");
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
            onChange={(e) => setText(e.target.value)}
            placeholder='[{ "name": "posts", "type": "base", "fields": [...] }]'
            rows={12}
            className="font-mono text-xs"
            aria-invalid={parseError ? true : undefined}
          />
          {parseError ? (
            <p className="flex items-center gap-1 text-xs font-medium text-destructive">
              <AlertCircle className="size-3 shrink-0" />
              {parseError}
            </p>
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
                Matches PocketBase's "delete missing" import option. Off by default so a partial export never
                drops the rest of your schema.
              </span>
            </span>
          </label>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" onClick={handleImport} disabled={!text.trim() || importMutation.isPending}>
            {importMutation.isPending ? <Spinner /> : null}
            {importMutation.isPending ? "Importing…" : "Import"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
