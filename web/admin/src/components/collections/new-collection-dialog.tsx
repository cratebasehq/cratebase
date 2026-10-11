import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { AlertCircle, Database, Users } from "lucide-react";
import { cn } from "@/lib/utils";
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
import { cb, describeFailure } from "@/lib/api";
import { defaultTimestampFields } from "@/lib/field-types";

const TYPE_CARDS = [
  {
    value: "base",
    label: "Base",
    icon: Database,
    description: "A plain table of records — posts, products, orders.",
  },
  {
    value: "auth",
    label: "Auth",
    icon: Users,
    description: "Records that can sign in: email or username, password, tokens, OAuth2.",
  },
] as const;

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
      cb.admin.collections.create({
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
        searchLanguage: value.searchLanguage,
        ownerField: value.ownerField,
        ...(value.type === "auth" && value.auth ? authOptionsPayload(value.auth, value.identityFields) : {}),
      }),
    onSuccess: async (created) => {
      await queryClient.invalidateQueries({ queryKey: ["collections"] });
      toast.success(`Collection "${created.name}" created`);
      onOpenChange(false);
      void navigate({ to: "/collections/$name", params: { name: created.name } });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      const fieldMessages = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
      toast.error(failure.title, {
        description: fieldMessages.length > 0 ? fieldMessages.join("; ") : failure.serverMessage || failure.detail,
      });
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
            <fieldset className="mb-6 flex flex-col gap-2">
              <legend className="mb-1.5 text-sm font-medium text-foreground">Collection type</legend>
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                {TYPE_CARDS.map((card) => {
                  const selected = value.type === card.value;
                  return (
                    <label
                      key={card.value}
                      className={cn(
                        "relative flex cursor-pointer flex-col gap-1 rounded-lg border p-3 transition-[border-color,background-color,box-shadow] duration-fast ease-out-strong has-[:focus-visible]:ring-3 has-[:focus-visible]:ring-ring/40",
                        selected ? "border-primary bg-brand-soft shadow-xs" : "border-border bg-card hover:border-border-strong",
                      )}
                    >
                      <input
                        type="radio"
                        name="collection-type"
                        value={card.value}
                        checked={selected}
                        onChange={() => {
                          if (!selected) setValue({ ...value, type: card.value, auth: emptyCollectionForm(card.value).auth });
                        }}
                        className="sr-only"
                      />
                      <span className="flex items-center gap-2 text-sm font-medium text-foreground">
                        <card.icon className={cn("size-4", selected ? "text-brand-text" : "text-muted-foreground")} />
                        {card.label}
                      </span>
                      <span className="text-xs text-muted-foreground">{card.description}</span>
                    </label>
                  );
                })}
              </div>
            </fieldset>
            <CollectionForm value={value} onChange={setValue} otherCollections={collections} isNew />
          </div>

          <SheetFooter className="flex-row items-center justify-end gap-3 border-t border-border">
            {errors.length > 0 ? (
              <span className="mr-auto flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                <AlertCircle className="size-3.5 shrink-0" />
                <span className="truncate" title={errors.join("\n")}>
                  {errors[0]}
                  {errors.length > 1 ? ` (+${errors.length - 1} more)` : ""}
                </span>
              </span>
            ) : null}
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
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
