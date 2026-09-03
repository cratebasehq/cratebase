import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useNavigate } from "@tanstack/react-router";
import { Database, ShieldUser } from "lucide-react";
import { Drawer } from "@/components/interior/drawer";
import { LoadingButton } from "@/components/interior/loading-button";
import { CollectionForm, emptyCollectionForm, type CollectionFormValue } from "@/components/collections/collection-form";
import { useCollections } from "@/hooks/use-collections";
import { cb } from "@/lib/api";

interface NewCollectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const TYPE_CARDS = [
  {
    type: "base" as const,
    icon: Database,
    title: "Base collection",
    description: "Plain records — posts, products, orders. No built-in auth.",
  },
  {
    type: "auth" as const,
    icon: ShieldUser,
    title: "Auth collection",
    description: "Records that can sign in — users, members, players.",
  },
];

export function NewCollectionDialog({ open, onOpenChange }: NewCollectionDialogProps) {
  const [step, setStep] = useState<"choose" | "form">("choose");
  const [value, setValue] = useState<CollectionFormValue>(emptyCollectionForm());
  const { data: collections = [] } = useCollections();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  useEffect(() => {
    if (open) {
      setStep("choose");
      setValue(emptyCollectionForm());
    }
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
        authOptions: value.type === "auth" ? { identityField: value.identityField } : undefined,
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
    <Drawer
      open={open}
      onOpenChange={onOpenChange}
      title={step === "choose" ? "New collection" : `New ${value.type} collection`}
      width={480}
    >
      {step === "choose" ? (
        <div className="flex flex-col gap-3">
          <p className="text-[13px] text-muted-foreground">What kind of data will this collection hold?</p>
          {TYPE_CARDS.map(({ type, icon: Icon, title, description }) => (
            <button
              key={type}
              type="button"
              onClick={() => {
                setValue(emptyCollectionForm(type));
                setStep("form");
              }}
              className="flex items-start gap-3 rounded-xl border border-border p-4 text-left transition-colors hover:border-primary hover:bg-accent/50"
            >
              <Icon className="mt-0.5 size-5 shrink-0 text-primary" />
              <span className="flex flex-col gap-0.5">
                <span className="text-[14px] font-medium text-foreground">{title}</span>
                <span className="text-[12.5px] text-muted-foreground">{description}</span>
              </span>
            </button>
          ))}
        </div>
      ) : (
        <>
          <button
            type="button"
            onClick={() => setStep("choose")}
            className="mb-4 text-[12.5px] font-medium text-muted-foreground transition-colors hover:text-foreground"
          >
            ← Change type
          </button>
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
        </>
      )}
    </Drawer>
  );
}
