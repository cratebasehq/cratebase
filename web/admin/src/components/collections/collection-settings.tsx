import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { CollectionModel } from "cratebase";
import { LoadingButton } from "@/components/interior/loading-button";
import { HoldToConfirm } from "@/components/interior/hold-to-confirm";
import {
  CollectionForm,
  collectionToFormValue,
  type CollectionFormValue,
} from "@/components/collections/collection-form";
import { useCollections } from "@/hooks/use-collections";
import { cb } from "@/lib/api";

export function CollectionSettings({ collection }: { collection: CollectionModel }) {
  const { data: collections = [] } = useCollections();
  const [value, setValue] = useState<CollectionFormValue>(() => collectionToFormValue(collection));
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
        schema: value.schema,
        listRule: value.listRule,
        viewRule: value.viewRule,
        createRule: value.createRule,
        updateRule: value.updateRule,
        deleteRule: value.deleteRule,
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

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-6">
      <CollectionForm value={value} onChange={setValue} otherCollections={collections.filter((c) => c.id !== collection.id)} isNew={false} />

      <div className="flex justify-end">
        <LoadingButton
          onAction={() => save.mutateAsync()}
          pendingLabel="Saving…"
          successLabel="Saved"
          errorLabel="Failed"
          className="!border-primary !bg-primary !px-4 !text-primary-foreground hover:!bg-primary/90 dark:!border-primary dark:!bg-primary dark:!text-primary-foreground dark:hover:!bg-primary/90"
        >
          Save changes
        </LoadingButton>
      </div>

      {collection.name === "users" ? (
        <div className="rounded-xl border border-border bg-secondary/30 p-4">
          <p className="text-[13px] font-medium text-foreground">Protected collection</p>
          <p className="mt-1 text-[12.5px] text-muted-foreground">
            <code className="font-mono">users</code> is provisioned automatically on first boot as the
            default auth collection every project needs a working sign-in table for. Deleting it here
            wouldn't recreate itself until the server restarts, breaking sign-in in the meantime — it
            can't be removed from the dashboard. Renaming the identity field or field schema is still
            fine above; only deletion is blocked.
          </p>
        </div>
      ) : (
        <div className="rounded-xl border border-destructive/30 bg-destructive/[0.03] p-4">
          <p className="text-[13px] font-medium text-destructive">Danger zone</p>
          <p className="mt-1 text-[12.5px] text-muted-foreground">
            Deleting a collection permanently removes it and every record in it. This cannot be undone.
          </p>
          <div className="mt-3">
            <HoldToConfirm
              onConfirm={() => remove.mutate()}
              confirmLabel="Deleted"
              className="!border-destructive/40 !text-destructive"
            >
              Hold to delete collection
            </HoldToConfirm>
          </div>
        </div>
      )}
    </div>
  );
}
