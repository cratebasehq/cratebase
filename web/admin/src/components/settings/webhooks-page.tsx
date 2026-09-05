import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Plus, Trash2, Webhook } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const EVENT_KINDS = ["create", "update", "delete"] as const;

/** A `_webhooks` record, as `crates/server/src/webhooks.rs` reads/writes
 * it — `events` is a comma-separated subset of `create`/`update`/`delete`,
 * not an array, matching the server's own `validate_events`. */
interface WebhookRecord {
  id: string;
  name: string;
  collectionRef: string;
  events: string;
  url: string;
  secret?: string;
  enabled: boolean;
  lastTriggeredAt?: string;
  lastStatus?: string;
  lastMessage?: string;
}

/**
 * CRUD over `_webhooks` — an outgoing POST to a configured URL whenever a
 * record event fires on a chosen collection. Same trust tier and CRUD
 * shape as `_cron_jobs` (see `CronJobsPage`): superuser-only end to end,
 * dispatched entirely server-side, this page only edits the row.
 */
export function WebhooksPage() {
  const queryClient = useQueryClient();
  const { data: collections } = useCollections();
  const [dialogWebhook, setDialogWebhook] = useState<WebhookRecord | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<WebhookRecord | null>(null);

  const { data, isLoading, error } = useQuery({
    queryKey: ["webhooks"],
    queryFn: () => cb.collection("_webhooks").getFullList<WebhookRecord>({ sort: "-created", requestKey: null }),
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_webhooks").delete(id),
    onSuccess: () => {
      toast.success("Webhook deleted");
      void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const webhooks = data ?? [];

  const item = settingsItemFor("/settings/webhooks")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Button size="sm" className="gap-1.5" onClick={() => setDialogWebhook("new")}>
          <Plus className="size-3.5" />
          New webhook
        </Button>
      }
    >

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Webhook />
            </EmptyMedia>
            <EmptyTitle>Couldn't load webhooks</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 2 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : webhooks.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Webhook />
            </EmptyMedia>
            <EmptyTitle>No webhooks yet</EmptyTitle>
            <EmptyDescription>Add one to notify an external service on record changes.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Webhook</TableHead>
              <TableHead>Collection</TableHead>
              <TableHead>Events</TableHead>
              <TableHead>Last delivery</TableHead>
              <TableHead className="w-24 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {webhooks.map((webhook) => (
              <TableRow key={webhook.id} className="align-top">
                <TableCell className="py-3">
                  <div className="flex items-center gap-2 font-medium">
                    {webhook.name}
                    {!webhook.enabled ? (
                      <Badge variant="outline" className="font-normal text-muted-foreground">
                        Disabled
                      </Badge>
                    ) : null}
                  </div>
                  <div className="mt-1 max-w-sm truncate font-mono text-[11px] text-muted-foreground/70">
                    {webhook.url}
                  </div>
                </TableCell>
                <TableCell className="py-3 text-sm">
                  {collections?.find((c) => c.id === webhook.collectionRef || c.name === webhook.collectionRef)
                    ?.name ?? webhook.collectionRef}
                </TableCell>
                <TableCell className="py-3 text-sm">
                  {webhook.events
                    .split(",")
                    .map((e) => e.trim())
                    .filter(Boolean)
                    .join(", ")}
                </TableCell>
                <TableCell className="py-3 text-xs text-muted-foreground">
                  {webhook.lastTriggeredAt ? (
                    <>
                      <Badge
                        variant="outline"
                        className={
                          webhook.lastStatus === "error" ? "font-normal text-destructive" : "font-normal text-success"
                        }
                      >
                        {webhook.lastStatus ?? "unknown"}
                      </Badge>
                      <div className="mt-1">{parseServerDate(webhook.lastTriggeredAt).toLocaleString()}</div>
                      {webhook.lastMessage ? <div className="max-w-xs truncate">{webhook.lastMessage}</div> : null}
                    </>
                  ) : (
                    "Never triggered"
                  )}
                </TableCell>
                <TableCell className="py-3 text-right">
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Edit ${webhook.name}`}
                      onClick={() => setDialogWebhook(webhook)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Delete ${webhook.name}`}
                      onClick={() => setDeleteTarget(webhook)}
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

      {dialogWebhook ? (
        <WebhookDialog
          webhook={dialogWebhook === "new" ? null : dialogWebhook}
          collections={collections ?? []}
          onOpenChange={(open) => {
            if (!open) setDialogWebhook(null);
          }}
        />
      ) : null}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete "{deleteTarget?.name}"?</DialogTitle>
            <DialogDescription>This stops delivery immediately. There is no undo.</DialogDescription>
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

/** Create or edit one `_webhooks` record. `webhook === null` means create. */
function WebhookDialog({
  webhook,
  collections,
  onOpenChange,
}: {
  webhook: WebhookRecord | null;
  collections: { id: string; name: string }[];
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(webhook?.name ?? "");
  const [collectionRef, setCollectionRef] = useState(webhook?.collectionRef ?? "");
  const [events, setEvents] = useState<Set<string>>(
    new Set(webhook ? webhook.events.split(",").map((e) => e.trim()).filter(Boolean) : ["create", "update", "delete"]),
  );
  const [url, setUrl] = useState(webhook?.url ?? "");
  const [secret, setSecret] = useState("");
  const [enabled, setEnabled] = useState(webhook?.enabled ?? true);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const save = useMutation({
    mutationFn: () => {
      const body: Record<string, unknown> = {
        name,
        collectionRef,
        events: Array.from(events).join(","),
        url,
        enabled,
      };
      // A blank secret box means "leave what's stored alone" on edit, and
      // "no signing" on create — same write-only convention as SMTP/S3.
      if (secret) body.secret = secret;
      return webhook ? cb.collection("_webhooks").update(webhook.id, body) : cb.collection("_webhooks").create(body);
    },
    onSuccess: () => {
      toast.success(webhook ? "Webhook updated" : "Webhook created");
      void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(described.title, { description: described.detail });
      }
    },
  });

  function toggleEvent(kind: string, checked: boolean) {
    setEvents((prev) => {
      const next = new Set(prev);
      if (checked) next.add(kind);
      else next.delete(kind);
      return next;
    });
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{webhook ? "Edit webhook" : "New webhook"}</DialogTitle>
          <DialogDescription>Fires after the record write already succeeded — this never blocks it.</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-name">Name</Label>
            <Input id="webhook-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Slack notify" />
            {fieldErrors["name"] ? <p className="text-xs text-destructive">{fieldErrors["name"]}</p> : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-collection">Collection</Label>
            <Select value={collectionRef} onValueChange={setCollectionRef}>
              <SelectTrigger id="webhook-collection" className="w-full">
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
            {fieldErrors["collectionRef"] ? (
              <p className="text-xs text-destructive">{fieldErrors["collectionRef"]}</p>
            ) : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>Events</Label>
            <div className="flex items-center gap-4">
              {EVENT_KINDS.map((kind) => (
                <div key={kind} className="flex items-center gap-1.5">
                  <Checkbox
                    id={`webhook-event-${kind}`}
                    checked={events.has(kind)}
                    onCheckedChange={(checked) => toggleEvent(kind, checked === true)}
                  />
                  <label htmlFor={`webhook-event-${kind}`} className="cursor-pointer text-sm capitalize">
                    {kind}
                  </label>
                </div>
              ))}
            </div>
            {fieldErrors["events"] ? <p className="text-xs text-destructive">{fieldErrors["events"]}</p> : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-url">URL</Label>
            <Input
              id="webhook-url"
              className="font-mono text-sm"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://example.com/hooks/cratebase"
            />
            {fieldErrors["url"] ? <p className="text-xs text-destructive">{fieldErrors["url"]}</p> : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-secret">Secret</Label>
            <Input
              id="webhook-secret"
              type="password"
              autoComplete="new-password"
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
              placeholder={webhook ? "Stored — leave blank to keep" : "Optional — enables HMAC signing"}
            />
            {fieldErrors["secret"] ? <p className="text-xs text-destructive">{fieldErrors["secret"]}</p> : null}
          </div>

          <div className="flex items-center gap-2">
            <Switch id="webhook-enabled" checked={enabled} onCheckedChange={setEnabled} />
            <Label htmlFor="webhook-enabled">Enabled</Label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            disabled={save.isPending || !collectionRef || events.size === 0}
            onClick={() => save.mutate()}
          >
            {save.isPending ? <Spinner className="size-3.5" /> : null}
            {webhook ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
