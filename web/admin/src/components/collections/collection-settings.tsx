import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useBlocker, useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { AlertCircle, RefreshCw, Undo2 } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { HoldToConfirm } from "@/components/ui/hold-to-confirm";
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
        ...(value.type === "auth" && value.auth ? authOptionsPayload(value.auth, value.identityField) : {}),
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

  function discard() {
    setValue(collectionToFormValue(baseline));
    setConfirmDiscard(false);
  }

  return (
    <div className="relative mx-auto flex w-full max-w-5xl flex-col gap-8 p-page pb-24">
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
        <div className="rounded-lg border border-destructive/30 bg-destructive/[0.03] p-4">
          <p className="text-sm font-medium text-destructive">Danger zone</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Deleting a collection permanently removes it and every record in it. This cannot be undone.
          </p>
          <div className="mt-3">
            <HoldToConfirm onConfirm={() => remove.mutate()} confirmLabel="Deleted">
              Hold to delete collection
            </HoldToConfirm>
          </div>
        </div>
      )}

      {/* The save bar only exists when there is something to save, and says
          why it can't when the form is invalid — the old one was always
          there and always enabled. */}
      {dirty ? (
        <div className="sticky bottom-0 -mx-page -mb-24 flex flex-wrap items-center gap-3 border-t border-border bg-background/95 px-page py-2.5 backdrop-blur-sm">
          <span className="text-sm text-muted-foreground">Unsaved changes</span>
          {errors.length > 0 ? (
            <span className="flex min-w-0 items-center gap-1.5 text-sm text-destructive">
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
            title={errors.length > 0 ? errors.join("\n") : undefined}
          >
            {save.isPending ? <Spinner /> : null}
            {save.isPending ? "Saving…" : "Save changes"}
          </Button>
        </div>
      ) : null}

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
