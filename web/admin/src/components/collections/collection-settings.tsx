import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useBlocker, useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { AlertCircle, ChevronUp, RefreshCw, Trash2, Undo2 } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Kbd } from "@/components/ui/kbd";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
import { CollectionForm } from "@/components/collections/collection-form";
import {
  authOptionsPayload,
  collectionFormErrors,
  collectionToFormValue,
  type CollectionFormValue,
} from "@/lib/collection-form-value";
import { useCollections } from "@/hooks/use-collections";
import { cb, describeFailure } from "@/lib/api";
import { managedFields } from "@/lib/field-types";
import { cn } from "@/lib/utils";

interface Change {
  text: string;
  /** `danger` = data loss on save, `warn` = behaviour change, `neutral` = cosmetic/additive. */
  tone: "danger" | "warn" | "neutral";
}

const RULE_LABELS = [
  ["listRule", "list"],
  ["viewRule", "view"],
  ["createRule", "create"],
  ["updateRule", "update"],
  ["deleteRule", "delete"],
] as const;

/** Human summary of what saving would change, derived by comparing the
 * edited form to the last saved one. Fields are matched by id so a rename
 * reads as a rename rather than a remove + add. */
function diffChanges(base: CollectionFormValue, next: CollectionFormValue): Change[] {
  const out: Change[] = [];
  const baseById = new Map(base.schema.map((f) => [f.id, f]));
  const nextById = new Map(next.schema.map((f) => [f.id, f]));

  for (const f of next.schema) {
    const before = baseById.get(f.id);
    if (!before) {
      out.push({ text: `Add field ${f.name || "(unnamed)"} · ${f.type}`, tone: "neutral" });
    } else if (before.name !== f.name) {
      out.push({ text: `Rename field ${before.name} → ${f.name}`, tone: "warn" });
    } else if (JSON.stringify(before) !== JSON.stringify(f)) {
      out.push({ text: `Edit field ${f.name} options`, tone: before.type !== f.type ? "danger" : "neutral" });
    }
  }
  for (const f of base.schema) {
    if (!nextById.has(f.id)) out.push({ text: `Remove field ${f.name} and its data`, tone: "danger" });
  }
  const commonBefore = base.schema.filter((f) => nextById.has(f.id)).map((f) => f.id);
  const commonAfter = next.schema.filter((f) => baseById.has(f.id)).map((f) => f.id);
  if (commonBefore.join() !== commonAfter.join()) out.push({ text: "Reorder fields", tone: "neutral" });

  for (const [key, label] of RULE_LABELS) {
    if (base[key] !== next[key]) out.push({ text: `Change ${label} rule`, tone: "warn" });
  }
  for (const idx of next.indexes) {
    if (!base.indexes.includes(idx)) out.push({ text: `Add index ${idx.match(/INDEX\s+(?:IF NOT EXISTS\s+)?[`"[]?([\w.-]+)/i)?.[1] ?? "statement"}`, tone: "neutral" });
  }
  for (const idx of base.indexes) {
    if (!next.indexes.includes(idx)) out.push({ text: `Drop index ${idx.match(/INDEX\s+(?:IF NOT EXISTS\s+)?[`"[]?([\w.-]+)/i)?.[1] ?? "statement"}`, tone: "warn" });
  }
  if (base.searchLanguage !== next.searchLanguage) out.push({ text: "Change search language", tone: "neutral" });
  if (base.ownerField !== next.ownerField) out.push({ text: "Change owner field", tone: "neutral" });
  if (JSON.stringify(base.auth) !== JSON.stringify(next.auth) || base.identityFields.join() !== next.identityFields.join()) {
    out.push({ text: "Change auth options", tone: "warn" });
  }
  return out;
}

/** Two form values are "the same edit" when they serialise the same. Used
 * both for the dirty check and to decide whether a background refetch may
 * quietly adopt server state. */
function serialise(value: CollectionFormValue): string {
  return JSON.stringify(value);
}

export function CollectionSettings({
  collection,
  onDirtyChange,
  onOpenApiDocs,
}: {
  collection: CollectionModel;
  /** Reported upward so the screen around this form can guard its own tab
   * switch, which unmounts the form and would otherwise drop the edits. */
  onDirtyChange?: (dirty: boolean) => void;
  /** Switches the collection page to its API tab — the expanded version of
   * the reference this used to render inline. */
  onOpenApiDocs?: () => void;
}) {
  const { data: collections = [] } = useCollections();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  /** The server state this form was seeded from. Replaced only on an
   * explicit adopt/save — never by a background refetch. */
  const [baseline, setBaseline] = useState<CollectionModel>(collection);
  const [value, setValue] = useState<CollectionFormValue>(() => collectionToFormValue(collection));
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const dirty = serialise(value) !== serialise(collectionToFormValue(baseline));

  /**
   * The old editor re-seeded form state from a `useEffect` keyed on the
   * query object, so every background refetch — a window focus, a realtime
   * event, another tab saving — silently threw away whatever was being
   * typed. State is now re-seeded only when this is a *different*
   * collection; a newer version of the same one is offered, not applied.
   */
  const collectionId = collection.id;
  const seededId = useRef(collectionId);
  useEffect(() => {
    if (seededId.current === collectionId) return;
    seededId.current = collectionId;
    setBaseline(collection);
    setValue(collectionToFormValue(collection));
  }, [collectionId, collection]);

  /** True when the server has a newer version than the one being edited. */
  const staleBaseline = collection.updated !== baseline.updated && collection.id === baseline.id;

  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  // A refetch that arrives while nothing is being edited is free to land.
  useEffect(() => {
    if (!staleBaseline || dirty) return;
    setBaseline(collection);
    setValue(collectionToFormValue(collection));
  }, [staleBaseline, dirty, collection]);

  const errors = useMemo(
    () => collectionFormErrors(value, collections.filter((c) => c.id !== collection.id)),
    [value, collections, collection.id],
  );

  const changes = useMemo(
    () => (dirty ? diffChanges(collectionToFormValue(baseline), value) : []),
    [dirty, baseline, value],
  );
  const destructive = changes.filter((c) => c.tone === "danger").length;

  // ⌘S / Ctrl+S saves from anywhere on the page, the way an editor does.
  const saveRef = useRef<() => void>(() => {});
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        saveRef.current();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const persistedFieldNames = useMemo(
    () => new Set(baseline.fields.map((f) => f.name)),
    [baseline],
  );

  /** Leaving the route with unsaved edits loses them, so ask first — and
   * ask the browser to ask too, for a reload or a closed tab. */
  const blocker = useBlocker({
    shouldBlockFn: ({ current, next }) => dirty && current.pathname !== next.pathname,
    enableBeforeUnload: () => dirty,
    withResolver: true,
  });

  const save = useMutation({
    mutationFn: () =>
      cb.admin.collections.update(collection.id, {
        name: value.name,
        type: value.type,
        // Update replaces the whole `fields` array — splice the edited
        // fields back in around the id/created/updated/auth columns this
        // form never shows, or saving would delete them.
        fields: [...managedFields(baseline), ...value.schema],
        indexes: value.indexes,
        listRule: value.listRule,
        viewRule: value.viewRule,
        createRule: value.createRule,
        updateRule: value.updateRule,
        deleteRule: value.deleteRule,
        searchLanguage: value.searchLanguage,
        ownerField: value.ownerField,
        ...(value.type === "auth" && value.auth ? authOptionsPayload(value.auth, value.identityFields) : {}),
      }),
    onSuccess: async (saved) => {
      setBaseline(saved);
      setValue(collectionToFormValue(saved));
      await queryClient.invalidateQueries({ queryKey: ["collections"] });
      await queryClient.invalidateQueries({ queryKey: ["records", saved.name] });
      toast.success("Collection saved");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      const fieldMessages = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
      toast.error(failure.title, {
        description: fieldMessages.length > 0 ? fieldMessages.join("; ") : failure.serverMessage || failure.detail,
      });
    },
  });

  const remove = useMutation({
    mutationFn: () => cb.admin.collections.delete(collection.id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["collections"] });
      toast.success(`Collection "${collection.name}" deleted`);
      void navigate({ to: "/" });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail });
    },
  });

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (save.isPending || errors.length > 0) return;
    save.mutate();
  }

  useEffect(() => {
    saveRef.current = () => {
      if (dirty && !save.isPending && errors.length === 0) save.mutate();
    };
  });

  function discard() {
    setValue(collectionToFormValue(baseline));
    setConfirmDiscard(false);
  }

  return (
    <div className="relative mx-auto flex w-full max-w-5xl flex-col gap-8 p-3 pb-24 sm:p-page sm:pb-24">
      {staleBaseline && dirty ? (
        <div className="flex flex-wrap items-center gap-3 rounded-lg border border-warning/40 bg-warning/[0.06] px-3 py-2">
          <AlertCircle className="size-4 shrink-0 text-warning" />
          <p className="min-w-0 flex-1 text-sm">
            This collection changed on the server while you were editing it. Your edits are still here — saving will
            overwrite the newer version.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-control-sm shrink-0 gap-1.5"
            onClick={() => {
              setBaseline(collection);
              setValue(collectionToFormValue(collection));
            }}
          >
            <RefreshCw className="size-3.5" />
            Load theirs
          </Button>
        </div>
      ) : null}

      <form onSubmit={handleSubmit} noValidate className="flex flex-col gap-8">
        <CollectionForm
          value={value}
          onChange={setValue}
          otherCollections={collections.filter((c) => c.id !== collection.id)}
          isNew={false}
          persistedFieldNames={persistedFieldNames}
        />
      </form>

      <Button variant="link" size="sm" className="h-auto p-0" onClick={onOpenApiDocs}>
        API reference for this collection →
      </Button>

      {collection.name === "users" ? (
        <div className="rounded-lg border border-border bg-surface-sunken/60 p-4">
          <p className="text-sm font-medium text-foreground">Protected collection</p>
          <p className="mt-1 text-sm text-muted-foreground">
            <code className="font-mono">users</code> is provisioned automatically on first boot as the default auth
            collection every project needs a working sign-in table for. Deleting it here wouldn't recreate itself
            until the server restarts, breaking sign-in in the meantime — it can't be removed from the dashboard.
            Renaming the identity field or field schema is still fine above; only deletion is blocked.
          </p>
        </div>
      ) : (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/30 bg-destructive/[0.03] p-4">
          <div className="min-w-0">
            <p className="text-sm font-medium text-destructive">Delete this collection</p>
            <p className="mt-1 max-w-measure text-sm text-muted-foreground">
              Permanently removes <span className="font-mono">{collection.name}</span>, every record in it and its
              uploaded files. This cannot be undone.
            </p>
          </div>
          <Button type="button" variant="destructive" size="sm" className="gap-1.5" onClick={() => setConfirmDelete(true)}>
            <Trash2 className="size-3.5" />
            Delete collection…
          </Button>
        </div>
      )}

      {/* The save bar only exists when there is something to save, and says
          why it can't when the form is invalid — the old one was always
          there and always enabled. */}
      {dirty ? (
        <div className="animate-rise sticky bottom-0 z-sticky -mx-3 -mb-24 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-border bg-background/95 px-3 py-2.5 shadow-e2 backdrop-blur-sm sm:-mx-page sm:px-page">
          <Popover>
            <PopoverTrigger asChild>
              <button
                type="button"
                className="flex h-control-sm items-center gap-1.5 rounded-md px-2 text-sm font-medium text-foreground transition-colors duration-fast hover:bg-accent"
              >
                <span className="size-1.5 rounded-full bg-warning" aria-hidden />
                <span className="font-tabular">{changes.length}</span> unsaved {changes.length === 1 ? "change" : "changes"}
                <ChevronUp className="size-3.5 text-muted-foreground" />
              </button>
            </PopoverTrigger>
            <PopoverContent side="top" align="start" className="w-80 p-2">
              <ul className="flex max-h-72 flex-col gap-0.5 overflow-y-auto">
                {changes.map((change, i) => (
                  <li
                    key={i}
                    className={cn(
                      "flex items-start gap-2 rounded px-2 py-1 text-xs",
                      change.tone === "danger" && "text-destructive",
                      change.tone === "warn" && "text-foreground",
                      change.tone === "neutral" && "text-muted-foreground",
                    )}
                  >
                    <span
                      aria-hidden
                      className={cn(
                        "mt-1.5 size-1.5 shrink-0 rounded-full",
                        change.tone === "danger" ? "bg-destructive" : change.tone === "warn" ? "bg-warning" : "bg-border-strong",
                      )}
                    />
                    <span className="min-w-0 break-words">{change.text}</span>
                  </li>
                ))}
              </ul>
            </PopoverContent>
          </Popover>
          {destructive > 0 ? (
            <span className="flex items-center gap-1.5 text-xs font-medium text-destructive">
              <AlertCircle className="size-3.5 shrink-0" />
              Destroys data on save
            </span>
          ) : null}
          {errors.length > 0 ? (
            <span role="alert" className="flex min-w-0 items-center gap-1.5 text-sm text-destructive">
              <AlertCircle className="size-3.5 shrink-0" />
              <span className="truncate" title={errors.join("\n")}>
                {errors[0]}
                {errors.length > 1 ? ` (+${errors.length - 1} more)` : ""}
              </span>
            </span>
          ) : null}
          <div className="flex-1" />
          <Button type="button" variant="ghost" size="sm" className="gap-1.5" onClick={() => setConfirmDiscard(true)}>
            <Undo2 className="size-3.5" />
            Discard
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={save.isPending || errors.length > 0}
            onClick={() => save.mutate()}
            title={errors.length > 0 ? errors.join("\n") : "Save (⌘S)"}
          >
            {save.isPending ? <Spinner /> : null}
            {save.isPending ? "Saving…" : "Save changes"}
            {!save.isPending ? <Kbd className="ml-1 hidden sm:inline-flex">⌘S</Kbd> : null}
          </Button>
        </div>
      ) : null}

      <TypedConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={`Delete ${collection.name}?`}
        description="This permanently deletes the collection, every record in it and its files. Anything calling its API will start failing."
        confirmValue={collection.name}
        confirmLabel="Delete collection"
        pendingLabel="Deleting…"
        pending={remove.isPending}
        onConfirm={() => remove.mutate()}
      />

      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard your changes?</AlertDialogTitle>
            <AlertDialogDescription>
              The form goes back to the last saved version of{" "}
              <span className="font-mono">{collection.name}</span>. Nothing on the server changes.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={discard}>
              Discard changes
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={blocker.status === "blocked"} onOpenChange={(open) => !open && blocker.reset?.()}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Leave with unsaved changes?</AlertDialogTitle>
            <AlertDialogDescription>
              Your edits to <span className="font-mono">{collection.name}</span>'s schema haven't been saved. Leaving
              this page loses them.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => blocker.reset?.()}>Stay</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => blocker.proceed?.()}>
              Leave and lose them
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
