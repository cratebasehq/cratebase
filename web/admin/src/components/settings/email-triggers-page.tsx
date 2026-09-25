import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Plus, Trash2, Zap } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { RuleField } from "@/components/collections/rule-field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const EVENT_KINDS = ["create", "update", "delete"] as const;

/** An `_emailTriggers` record, as `crates/server/src/email_triggers.rs`
 * reads/writes it. */
interface TriggerRecord {
  id: string;
  collection: string;
  event: (typeof EVENT_KINDS)[number];
  template: string;
  toField: string;
  condition: string | null;
  enabled: boolean;
  dataMap: Record<string, unknown> | null;
}

/**
 * CRUD over `_emailTriggers` — send an `_emailTemplates` template
 * automatically when a record event fires, entirely from the dashboard.
 * Same trust tier and CRUD shape as `_webhooks` (see `WebhooksPage`):
 * superuser-only end to end, dispatched entirely server-side.
 */
export function EmailTriggersPage() {
  const queryClient = useQueryClient();
  const { data: collections } = useCollections();
  const [dialogTrigger, setDialogTrigger] = useState<TriggerRecord | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<TriggerRecord | null>(null);

  const { data, isLoading, error } = useQuery({
    queryKey: ["email-triggers"],
    queryFn: () => cb.collection("_emailTriggers").fullList({ sort: "-created" }) as unknown as Promise<TriggerRecord[]>,
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_emailTriggers").delete(id),
    onSuccess: () => {
      toast.success("Trigger deleted");
      void queryClient.invalidateQueries({ queryKey: ["email-triggers"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const triggers = data ?? [];
  const item = settingsItemFor("/settings/email-triggers")!;

  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Button size="sm" className="gap-1.5" onClick={() => setDialogTrigger("new")}>
          <Plus className="size-3.5" />
          New trigger
        </Button>
      }
    >
      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Zap />
            </EmptyMedia>
            <EmptyTitle>Couldn't load triggers</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 2 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : triggers.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Zap />
            </EmptyMedia>
            <EmptyTitle>No email triggers yet</EmptyTitle>
            <EmptyDescription>Send a template automatically when a record is created, updated, or deleted.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Trigger</TableHead>
              <TableHead>Collection / event</TableHead>
              <TableHead>To</TableHead>
              <TableHead>Condition</TableHead>
              <TableHead className="w-24 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {triggers.map((trigger) => (
              <TableRow key={trigger.id} className="align-top">
                <TableCell className="py-3">
                  <div className="flex items-center gap-2 font-medium">
                    {trigger.template}
                    {!trigger.enabled ? (
                      <Badge variant="outline" className="font-normal text-muted-foreground">
                        Disabled
                      </Badge>
                    ) : null}
                  </div>
                </TableCell>
                <TableCell className="py-3 text-sm">
                  {collections?.find((c) => c.id === trigger.collection || c.name === trigger.collection)?.name ??
                    trigger.collection}{" "}
                  · {trigger.event}
                </TableCell>
                <TableCell className="py-3 font-mono text-xs text-muted-foreground">{trigger.toField}</TableCell>
                <TableCell className="max-w-xs truncate py-3 font-mono text-xs text-muted-foreground">
                  {trigger.condition || <span className="italic">always</span>}
                </TableCell>
                <TableCell className="py-3 text-right">
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Edit ${trigger.template} trigger`}
                      onClick={() => setDialogTrigger(trigger)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Delete ${trigger.template} trigger`}
                      onClick={() => setDeleteTarget(trigger)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {dialogTrigger ? (
        <TriggerDialog
          trigger={dialogTrigger === "new" ? null : dialogTrigger}
          collections={collections ?? []}
          onOpenChange={(open) => {
            if (!open) setDialogTrigger(null);
          }}
        />
      ) : null}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete this trigger?</DialogTitle>
            <DialogDescription>This stops the automatic send immediately. There is no undo.</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => deleteTarget && remove.mutate(deleteTarget.id)}
            >
              {remove.isPending ? <Spinner className="size-3.5" /> : null}
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}

/** Create or edit one `_emailTriggers` record. `trigger === null` means create. */
function TriggerDialog({
  trigger,
  collections,
  onOpenChange,
}: {
  trigger: TriggerRecord | null;
  collections: { id: string; name: string; fields?: { name: string }[] }[];
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [collection, setCollection] = useState(trigger?.collection ?? "");
  const [event, setEvent] = useState<(typeof EVENT_KINDS)[number]>(trigger?.event ?? "create");
  const [template, setTemplate] = useState(trigger?.template ?? "");
  const [toField, setToField] = useState(trigger?.toField ?? "");
  const [condition, setCondition] = useState<string | null>(trigger?.condition ?? null);
  const [enabled, setEnabled] = useState(trigger?.enabled ?? true);
  const [dataMap, setDataMap] = useState(trigger?.dataMap ? JSON.stringify(trigger.dataMap, null, 2) : "");
  const [dataMapError, setDataMapError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const { data: templates } = useQuery({
    queryKey: ["email-templates", "picker"],
    queryFn: () =>
      cb.collection("_emailTemplates").fullList({ sort: "key" }) as unknown as Promise<{ key: string; name: string }[]>,
  });

  const targetFields = collections.find((c) => c.name === collection || c.id === collection)?.fields ?? [];

  const save = useMutation({
    mutationFn: () => {
      let parsedDataMap: unknown = null;
      if (dataMap.trim()) {
        try {
          parsedDataMap = JSON.parse(dataMap);
          setDataMapError(null);
        } catch {
          setDataMapError("Not valid JSON.");
          throw new Error("invalid dataMap");
        }
      }
      const body: Record<string, unknown> = {
        collection,
        event,
        template,
        toField,
        condition,
        enabled,
        dataMap: parsedDataMap,
      };
      return trigger
        ? cb.collection("_emailTriggers").update(trigger.id, body)
        : cb.collection("_emailTriggers").create(body);
    },
    onSuccess: () => {
      toast.success(trigger ? "Trigger updated" : "Trigger created");
      void queryClient.invalidateQueries({ queryKey: ["email-triggers"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      if (failure instanceof Error && failure.message === "invalid dataMap") return;
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(described.title, { description: described.detail });
      }
    },
  });

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{trigger ? "Edit trigger" : "New trigger"}</DialogTitle>
          <DialogDescription>Fires after the record write already succeeded — this never blocks it.</DialogDescription>
        </DialogHeader>

        <div className="flex max-h-[70vh] flex-col gap-4 overflow-y-auto pr-1">
          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="trigger-collection">Collection</Label>
              <Select value={collection} onValueChange={setCollection}>
                <SelectTrigger id="trigger-collection" className="w-full">
                  <SelectValue placeholder="Select a collection…" />
                </SelectTrigger>
                <SelectContent>
                  {collections.map((c) => (
                    <SelectItem key={c.id} value={c.name}>
                      {c.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {fieldErrors["collection"] ? <p className="text-xs text-destructive">{fieldErrors["collection"]}</p> : null}
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="trigger-event">Event</Label>
              <Select value={event} onValueChange={(v) => setEvent(v as (typeof EVENT_KINDS)[number])}>
                <SelectTrigger id="trigger-event" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {EVENT_KINDS.map((kind) => (
                    <SelectItem key={kind} value={kind} className="capitalize">
                      {kind}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="trigger-template">Template</Label>
            <Select value={template} onValueChange={setTemplate}>
              <SelectTrigger id="trigger-template" className="w-full">
                <SelectValue placeholder="Select a template…" />
              </SelectTrigger>
              <SelectContent>
                {(templates ?? []).map((t) => (
                  <SelectItem key={t.key} value={t.key}>
                    {t.name} ({t.key})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {fieldErrors["template"] ? <p className="text-xs text-destructive">{fieldErrors["template"]}</p> : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="trigger-to-field">Recipient (toField)</Label>
            <Input
              id="trigger-to-field"
              list="trigger-to-field-options"
              value={toField}
              onChange={(e) => setToField(e.target.value)}
              placeholder="e.g. email, customer.email, or ops@example.com"
              className="font-mono text-sm"
            />
            <datalist id="trigger-to-field-options">
              {targetFields.map((f) => (
                <option key={f.name} value={f.name} />
              ))}
            </datalist>
            <p className="text-xs text-muted-foreground">
              A field on the record holding the recipient's email, or a literal address.
            </p>
            {fieldErrors["toField"] ? <p className="text-xs text-destructive">{fieldErrors["toField"]}</p> : null}
          </div>

          <RuleField
            label="Condition"
            value={condition}
            onChange={setCondition}
            nullOption={{ label: "Always", description: "Fires on every matching event, no condition." }}
            publicOption={{ label: "Never", description: "Fires on nothing — effectively the same as disabling it." }}
            customLabel="When…"
          />

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="trigger-data-map">Extra template data (optional JSON)</Label>
            <Textarea
              id="trigger-data-map"
              value={dataMap}
              onChange={(e) => setDataMap(e.target.value)}
              placeholder={'{ "appEnv": "production" }'}
              className="min-h-20 font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">
              Merged onto the default <code className="font-mono">{"{ record: <written record> }"}</code> template data.
            </p>
            {dataMapError ? <p className="text-xs text-destructive">{dataMapError}</p> : null}
          </div>

          <div className="flex items-center gap-2">
            <Switch id="trigger-enabled" checked={enabled} onCheckedChange={setEnabled} />
            <Label htmlFor="trigger-enabled">Enabled</Label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={save.isPending || !collection || !template || !toField} onClick={() => save.mutate()}>
            {save.isPending ? <Spinner className="size-3.5" /> : null}
            {trigger ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
