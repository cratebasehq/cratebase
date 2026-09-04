import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useNavigate } from "@tanstack/react-router";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { CollectionForm } from "@/components/collections/collection-form";
import { authOptionsPayload, collectionFormErrors, emptyCollectionForm, type CollectionFormValue } from "@/lib/collection-form-value";
import { useCollections } from "@/hooks/use-collections";
import { cb } from "@/lib/api";
import { defaultTimestampFields } from "@/lib/field-types";

interface NewCollectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function NewCollectionDialog({ open, onOpenChange }: NewCollectionDialogProps) {
  const [value, setValue] = useState<CollectionFormValue>(emptyCollectionForm());
  const [pending, setPending] = useState(false);
  const { data: collections = [] } = useCollections();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  // Same gate as the schema editor: a field with no name or a relation
  // with no target is a 400 waiting to happen, so Create stays disabled
  // until the form would actually be accepted.
  const errors = collectionFormErrors(value, collections);

  useEffect(() => {
    if (open) {
      setValue(emptyCollectionForm());
      setPending(false);
    }
  }, [open]);

  const create = useMutation({
    mutationFn: () =>
      cb.collections.create({
        name: value.name,
        type: value.type,
        // `id` and, for auth collections, the auth columns are added by the
        // server on create — but not `created`/`updated`, so those go in
        // explicitly or the collection ends up with no timestamp columns.
        fields: [...defaultTimestampFields(), ...value.schema],
        indexes: value.indexes,
        listRule: value.listRule,
        viewRule: value.viewRule,
        createRule: value.createRule,
        updateRule: value.updateRule,
        deleteRule: value.deleteRule,
        ...(value.type === "auth" && value.auth ? authOptionsPayload(value.auth, value.identityField) : {}),
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

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending || errors.length > 0) return;
    setPending(true);
    try {
      await create.mutateAsync();
    } catch {
      // Surfaced as a toast by the mutation's own onError.
    } finally {
      setPending(false);
    }
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[680px]">
        <form onSubmit={handleSubmit} noValidate className="flex h-full min-h-0 flex-col">
          <SheetHeader>
            <SheetTitle>New collection</SheetTitle>
            <SheetDescription>Name it, define its fields, and set who can read and write it.</SheetDescription>
          </SheetHeader>

          <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
            <CollectionForm value={value} onChange={setValue} otherCollections={collections} isNew />
          </div>

          <SheetFooter className="flex-row justify-end border-t border-border">
            <Button type="submit" disabled={pending || errors.length > 0} title={errors[0]}>
              {pending ? <Spinner /> : null}
              {pending ? "Creating…" : "Create collection"}
            </Button>
          </SheetFooter>
        </form>
      </SheetContent>
    </Sheet>
  );
}
