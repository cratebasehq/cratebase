import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useNavigate } from "@tanstack/react-router";
import { Drawer } from "@/components/interior/drawer";
import { LoadingButton } from "@/components/interior/loading-button";
import { CollectionForm, emptyCollectionForm, type CollectionFormValue } from "@/components/collections/collection-form";
import { useCollections } from "@/hooks/use-collections";
import { cb } from "@/lib/api";

interface NewCollectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function NewCollectionDialog({ open, onOpenChange }: NewCollectionDialogProps) {
  const [value, setValue] = useState<CollectionFormValue>(emptyCollectionForm());
  const { data: collections = [] } = useCollections();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  useEffect(() => {
    if (open) setValue(emptyCollectionForm());
  }, [open]);

  const create = useMutation({
    mutationFn: () =>
      cb.collections.create({
        name: value.name,
        type: value.type,
        schema: value.schema,
        listRule: value.listRule,
        viewRule: value.viewRule,
        createRule: value.createRule,
        updateRule: value.updateRule,
        deleteRule: value.deleteRule,
      }),
    onSuccess: async (created) => {
      await queryClient.invalidateQueries({ queryKey: ["collections"] });
      toast.success(`Collection "${created.name}" created`);
      onOpenChange(false);
      void navigate({ to: "/collections/$name", params: { name: created.name } });
    },
    onError: (error) => {
      toast.error(error instanceof Error ? error.message : "Failed to create collection");
    },
  });

  return (
    <Drawer open={open} onOpenChange={onOpenChange} title="New collection" width={480}>
      <CollectionForm value={value} onChange={setValue} otherCollections={collections} isNew />
      <div className="mt-6 flex justify-end">
        <LoadingButton
          onAction={() => create.mutateAsync()}
          pendingLabel="Creating…"
          successLabel="Created"
          errorLabel="Failed"
          disabled={value.name.length === 0}
          className="!border-primary !bg-primary !px-4 !text-primary-foreground hover:!bg-primary/90 dark:!border-primary dark:!bg-primary dark:!text-primary-foreground dark:hover:!bg-primary/90"
        >
          Create collection
        </LoadingButton>
      </div>
    </Drawer>
  );
}
