import { useMemo, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowRight, Pencil, Plus, Trash2, Zap } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { RuleField } from "@/components/collections/rule-field";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { StatusBadge } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const EVENT_KINDS = ["create", "update", "delete"] as const;

const EVENT_PHRASE: Record<(typeof EVENT_KINDS)[number], string> = {
  create: "created",
  update: "updated",
  delete: "deleted",
};

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
  const [q, setQ] = useState("");
  const [dialogTrigger, setDialogTrigger] = useState<TriggerRecord | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<TriggerRecord | null>(null);

  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["email-triggers"],
    queryFn: () => cb.collection("_emailTriggers").fullList({ sort: "-created" }) as unknown as Promise<TriggerRecord[]>,
  });

  const toggle = useMutation({
    mutationFn: ({ trigger, enabled }: { trigger: TriggerRecord; enabled: boolean }) =>
      cb.collection("_emailTriggers").update(trigger.id, { enabled }),
    onSuccess: (_result, { trigger, enabled }) => {
      toast.success(`${enabled ? "Enabled" : "Disabled"} the “${trigger.template}” trigger`);
      void queryClient.invalidateQueries({ queryKey: ["email-triggers"] });
    },
    onError: (failure, { trigger }) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't update the “${trigger.template}” trigger`, {
        description: described.serverMessage || described.detail,
      });
    },
  });

  const remove = useMutation({
    mutationFn: (trigger: TriggerRecord) => cb.collection("_emailTriggers").delete(trigger.id),
    onSuccess: (_result, trigger) => {
      toast.success(`Deleted the “${trigger.template}” trigger`);
      void queryClient.invalidateQueries({ queryKey: ["email-triggers"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't delete the trigger", { description: described.serverMessage || described.detail });
    },
  });

  const triggers = useMemo(() => data ?? [], [data]);
  const visible = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return triggers;
    return triggers.filter((trigger) =>
      [trigger.template, trigger.collection, trigger.event, trigger.toField].some((text) =>
        text.toLowerCase().includes(needle),
      ),
    );
  }, [triggers, q]);
  const item = settingsItemFor("/settings/email-triggers")!;

  const newTriggerButton = (
    <Button size="sm" onClick={() => setDialogTrigger("new")}>
      <Plus />
      New trigger
    </Button>
  );

  function collectionName(value: string): string {
    return collections?.find((c) => c.id === value || c.name === value)?.name ?? value;
  }

  return (
    <SettingsPage title={item.label} description={item.description} width="wide" action={newTriggerButton}>
      <p className="max-w-measure text-xs text-pretty text-muted-foreground">
        A trigger watches one collection. When a record is created, updated or deleted — and the optional condition
        matches — the chosen template is emailed to the address in the recipient field. It runs after the write has
        succeeded and never blocks it.
      </p>

      {triggers.length > 0 ? (
        <DataTableToolbar
          search={{ value: q, onChange: setQ, placeholder: "Search by template, collection or event…" }}
          actions={
            <span className="font-tabular text-xs text-muted-foreground">
              {triggers.filter((trigger) => trigger.enabled).length} of {triggers.length} enabled
            </span>
          }
        />
      ) : null}

      {error && !data ? (
        <ErrorState title="Couldn't load triggers" error={error} onRetry={() => void refetch()} />
      ) : isPending ? (
        <ListSkeleton rows={3} />
      ) : triggers.length === 0 ? (
        <EmptyState
          compact
          icon={<Zap />}
          title="No email triggers yet"
          description="Send a template automatically when a record is created, updated or deleted — a welcome email on signup, an alert on a new order."
          action={newTriggerButton}
        />
      ) : visible.length === 0 ? (
        <EmptyState
          compact
          icon={<Zap />}
          title="No triggers match"
          description={`Nothing matches “${q}”.`}
          action={
            <Button variant="outline" size="sm" onClick={() => setQ("")}>
              Clear search
            </Button>
          }
        />
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-20">Enabled</TableHead>
                <TableHead>When</TableHead>
                <TableHead>Send template</TableHead>
                <TableHead>To</TableHead>
                <TableHead>Condition</TableHead>
                <TableHead className="w-24 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.map((trigger) => (
                <TableRow key={trigger.id} className={trigger.enabled ? undefined : "text-muted-foreground"}>
                  <TableCell>
                    <Switch
                      checked={trigger.enabled}
                      disabled={toggle.isPending && toggle.variables?.trigger.id === trigger.id}
                      aria-label={`${trigger.enabled ? "Disable" : "Enable"} the ${trigger.template} trigger`}
                      onCheckedChange={(enabled) => toggle.mutate({ trigger, enabled })}
                    />
                  </TableCell>
                  <TableCell className="text-sm">
                    A <span className="font-mono">{collectionName(trigger.collection)}</span> record is{" "}
                    <span className="font-medium">{EVENT_PHRASE[trigger.event] ?? trigger.event}</span>
                  </TableCell>
                  <TableCell>
                    <span className="inline-flex flex-wrap items-center gap-1.5">
                      <ArrowRight className="size-3 text-muted-foreground" aria-hidden />
                      <span className="font-mono text-sm">{trigger.template}</span>
                      {!trigger.enabled ? <StatusBadge tone="neutral">Disabled</StatusBadge> : null}
                    </span>
                  </TableCell>
                  <TableCell className="font-mono text-xs text-muted-foreground">{trigger.toField}</TableCell>
                  <TableCell className="max-w-56 truncate font-mono text-xs text-muted-foreground" title={trigger.condition ?? undefined}>
                    {trigger.condition || <span className="font-sans italic">always</span>}
                  </TableCell>
                  <TableCell>
                    <div className="flex justify-end gap-0.5">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Edit ${trigger.template} trigger`}
                        title="Edit"
                        onClick={() => setDialogTrigger(trigger)}
                      >
                        <Pencil />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Delete ${trigger.template} trigger`}
                        title="Delete"
                        onClick={() => setDeleteTarget(trigger)}
                      >
                        <Trash2 />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
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

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && !remove.isPending && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete the “{deleteTarget?.template}” trigger?</DialogTitle>
            <DialogDescription>
              {deleteTarget ? (
                <>
                  Emails for <span className="font-mono">{collectionName(deleteTarget.collection)}</span> records being{" "}
                  {EVENT_PHRASE[deleteTarget.event] ?? deleteTarget.event} stop immediately. The template itself is not
                  deleted. There is no undo.
                </>
              ) : null}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)} disabled={remove.isPending}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => deleteTarget && remove.mutate(deleteTarget)}
            >
              {remove.isPending ? <Spinner /> : null}
              Delete trigger
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}

/** One field's inline error, wired to its input via `aria-describedby`. */
function FieldError({ id, message }: { id: string; message?: string }) {
  if (!message) return null;
  return (
    <p id={id} role="alert" className="text-xs text-destructive">
      {message}
    </p>
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
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const { data: templates } = useQuery({
    queryKey: ["email-templates", "picker"],
    queryFn: () =>
      cb.collection("_emailTemplates").fullList({ sort: "key" }) as unknown as Promise<{ key: string; name: string }[]>,
  });

  const targetFields = collections.find((c) => c.name === collection || c.id === collection)?.fields ?? [];

  const save = useMutation({
    mutationFn: (parsedDataMap: unknown) => {
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
      toast.success(trigger ? `Updated the “${template}” trigger` : `Created a trigger for “${template}”`);
      void queryClient.invalidateQueries({ queryKey: ["email-triggers"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error("Couldn't save the trigger", { description: described.serverMessage || described.detail });
      }
    },
  });

  function submit(formEvent: FormEvent) {
    formEvent.preventDefault();
    if (save.isPending) return;
    const errors: Record<string, string> = {};
    if (!collection) errors.collection = "Pick the collection to watch.";
    if (!template) errors.template = "Pick the template to send.";
    if (!toField.trim()) errors.toField = "Name the field holding the recipient, or enter an address.";
    let parsedDataMap: unknown = null;
    if (dataMap.trim()) {
      try {
        parsedDataMap = JSON.parse(dataMap);
      } catch {
        errors.dataMap = "Not valid JSON.";
      }
    }
    setFieldErrors(errors);
    if (Object.keys(errors).length > 0) return;
    save.mutate(parsedDataMap);
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={submit} noValidate className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>{trigger ? "Edit trigger" : "New trigger"}</DialogTitle>
            <DialogDescription>Fires after the record write already succeeded — this never blocks it.</DialogDescription>
          </DialogHeader>

          <div className="flex max-h-[60vh] flex-col gap-4 overflow-y-auto px-0.5 pr-1">
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="trigger-collection">Collection</Label>
                <Select value={collection} onValueChange={setCollection}>
                  <SelectTrigger
                    id="trigger-collection"
                    className="w-full"
                    aria-invalid={fieldErrors.collection ? true : undefined}
                    aria-describedby={fieldErrors.collection ? "trigger-collection-error" : undefined}
                  >
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
                <FieldError id="trigger-collection-error" message={fieldErrors.collection} />
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
                <SelectTrigger
                  id="trigger-template"
                  className="w-full"
                  aria-invalid={fieldErrors.template ? true : undefined}
                  aria-describedby={fieldErrors.template ? "trigger-template-error" : "trigger-template-help"}
                >
                  <SelectValue placeholder="Select a template…" />
                </SelectTrigger>
                <SelectContent>
                  {(templates ?? []).map((t) => (
                    <SelectItem key={t.key} value={t.key}>
                      {t.name ? `${t.name} (${t.key})` : t.key}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldError id="trigger-template-error" message={fieldErrors.template} />
              <p id="trigger-template-help" className="text-xs text-muted-foreground">
                {templates && templates.length === 0 ? (
                  <>
                    No templates exist yet —{" "}
                    <Link
                      to="/settings/email-templates/$id"
                      params={{ id: "new" }}
                      className="text-brand-text hover:underline"
                    >
                      create one
                    </Link>{" "}
                    first.
                  </>
                ) : (
                  "The email that gets sent. Manage these under Templates."
                )}
              </p>
            </div>

            <div className="flex flex-col gap-1.5">
              <Label htmlFor="trigger-to-field">Recipient (toField)</Label>
              <Input
                id="trigger-to-field"
                list="trigger-to-field-options"
                value={toField}
                onChange={(e) => setToField(e.target.value)}
                placeholder="e.g. email, customer.email, or ops@example.com"
                aria-invalid={fieldErrors.toField ? true : undefined}
                aria-describedby={fieldErrors.toField ? "trigger-to-field-error" : "trigger-to-field-help"}
                className="font-mono text-sm"
              />
              <datalist id="trigger-to-field-options">
                {targetFields.map((f) => (
                  <option key={f.name} value={f.name} />
                ))}
              </datalist>
              <FieldError id="trigger-to-field-error" message={fieldErrors.toField} />
              <p id="trigger-to-field-help" className="text-xs text-muted-foreground">
                A field on the record holding the recipient's email, or a literal address.
              </p>
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
                aria-invalid={fieldErrors.dataMap ? true : undefined}
                aria-describedby={fieldErrors.dataMap ? "trigger-data-map-error" : "trigger-data-map-help"}
                className="min-h-20 font-mono text-xs"
              />
              <FieldError id="trigger-data-map-error" message={fieldErrors.dataMap} />
              <p id="trigger-data-map-help" className="text-xs text-muted-foreground">
                Merged onto the default <code className="font-mono">{"{ record: <written record> }"}</code> template
                data.
              </p>
            </div>

            <div className="flex items-center gap-2">
              <Switch id="trigger-enabled" checked={enabled} onCheckedChange={setEnabled} />
              <Label htmlFor="trigger-enabled">Enabled</Label>
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={save.isPending}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {save.isPending ? <Spinner /> : null}
              {trigger ? "Save trigger" : "Create trigger"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
