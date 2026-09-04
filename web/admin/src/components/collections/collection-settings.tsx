import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { CollectionModel } from "pocketbase";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { HoldToConfirm } from "@/components/ui/hold-to-confirm";
import {
  CollectionForm,
  collectionToFormValue,
  type CollectionFormValue,
} from "@/components/collections/collection-form";
import { useCollections } from "@/hooks/use-collections";
import { cb } from "@/lib/api";
import { managedFields } from "@/lib/field-types";

export function CollectionSettings({ collection }: { collection: CollectionModel }) {
  const { data: collections = [] } = useCollections();
  const [value, setValue] = useState<CollectionFormValue>(() => collectionToFormValue(collection));
  const [pending, setPending] = useState(false);
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  useEffect(() => {
    setValue(collectionToFormValue(collection));
  }, [collection]);

  const save = useMutation({
    mutationFn: () =>
      cb.collections.update(collection.id, {
        name: value.name,
        type: value.type,
        // Update replaces the whole `fields` array — splice the edited
        // fields back in around the id/created/updated/auth columns this
        // form never shows, or saving would delete them.
        fields: [...managedFields(collection), ...value.schema],
        listRule: value.listRule,
        viewRule: value.viewRule,
        createRule: value.createRule,
        updateRule: value.updateRule,
        deleteRule: value.deleteRule,
        ...(value.type === "auth" ? { passwordAuth: { identityFields: [value.identityField] } } : {}),
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["collections"] });
      toast.success("Collection saved");
    },
    onError: (error) => toast.error(error instanceof Error ? error.message : "Failed to save"),
  });

  const remove = useMutation({
    mutationFn: () => cb.collections.delete(collection.id),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["collections"] });
      toast.success(`Collection "${collection.name}" deleted`);
      void navigate({ to: "/" });
    },
    onError: (error) => toast.error(error instanceof Error ? error.message : "Failed to delete"),
  });

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    setPending(true);
    try {
      await save.mutateAsync();
    } catch {
      // Surfaced as a toast by the mutation's own onError.
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-6">
      <form onSubmit={handleSubmit} className="flex flex-col gap-8">
        <CollectionForm value={value} onChange={setValue} otherCollections={collections.filter((c) => c.id !== collection.id)} isNew={false} />

        <div className="flex justify-end">
          <Button type="submit" disabled={pending}>
            {pending ? <Spinner /> : null}
            {pending ? "Saving…" : "Save changes"}
          </Button>
        </div>
      </form>

      {collection.name === "users" ? (
        <div className="rounded-xl border border-border bg-secondary/30 p-4">
          <p className="text-sm font-medium text-foreground">Protected collection</p>
          <p className="mt-1 text-sm text-muted-foreground">
            <code className="font-mono">users</code> is provisioned automatically on first boot as the
            default auth collection every project needs a working sign-in table for. Deleting it here
            wouldn't recreate itself until the server restarts, breaking sign-in in the meantime — it
            can't be removed from the dashboard. Renaming the identity field or field schema is still
            fine above; only deletion is blocked.
          </p>
        </div>
      ) : (
        <div className="rounded-xl border border-destructive/30 bg-destructive/[0.03] p-4">
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
    </div>
  );
}
