import { Fragment, useState } from "react";
import { useMutation, useQuery, useQueryClient, type UseMutationResult } from "@tanstack/react-query";
import { Check, ChevronRight, Copy, Eye, EyeOff, Pause, Pencil, Play, Plus, RotateCw, Send, Sparkles, Trash2, Webhook } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { settingsItemFor } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatusBadge, type Tone } from "@/components/layout/stat-card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { LIVE_INTERVAL_MS, LiveToggle } from "@/components/settings/live-toggle";
import { SettingsPage } from "@/components/settings/settings-form";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
import {
  DetailBody,
  DetailCallout,
  DetailChip,
  DetailEmpty,
  DetailList,
  DetailRow,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
  DetailTabPanel,
  DetailTabs,
} from "@/components/layout/detail-sheet";
import { CodeBlock } from "@/components/settings/detail-json";
import { TimeAgo } from "@/components/settings/time-ago";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const EVENT_KINDS = [
  { value: "create", label: "Create", help: "A record was created" },
  { value: "update", label: "Update", help: "A record was changed" },
  { value: "delete", label: "Delete", help: "A record was removed" },
] as const;

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
  consecutiveFailures?: number;
  lastTriggeredAt?: string;
  lastStatus?: string;
  lastMessage?: string;
}

function eventsOf(webhook: Pick<WebhookRecord, "events">): string[] {
  return webhook.events
    .split(",")
    .map((e) => e.trim())
    .filter(Boolean);
}

function failureToast(failure: unknown) {
  const described = describeFailure(failure);
  toast.error(described.title, { description: described.detail });
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
  const [dialogWebhook, setDialogWebhook] = useState<"new" | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [tab, setTab] = useState("overview");
  const [deleteTarget, setDeleteTarget] = useState<WebhookRecord | null>(null);
  const [search, setSearch] = useState("");

  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["webhooks"],
    queryFn: () => cb.collection("_webhooks").fullList({ sort: "-created" }) as unknown as Promise<WebhookRecord[]>,
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_webhooks").delete(id),
    onSuccess: (_result, id) => {
      toast.success(`Webhook "${deleteTarget?.name ?? ""}" deleted`);
      void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
      setDeleteTarget(null);
      setSelectedId((current) => (current === id ? null : current));
    },
    onError: failureToast,
  });

  const toggle = useMutation({
    mutationFn: ({ webhook, enabled }: { webhook: WebhookRecord; enabled: boolean }) =>
      cb.collection("_webhooks").update(webhook.id, { enabled }),
    onSuccess: (_result, { webhook, enabled }) => {
      toast.success(`Webhook "${webhook.name}" ${enabled ? "enabled" : "disabled"}`);
      void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
    },
    onError: failureToast,
  });

  // Queues a `ping` delivery through the same durable worker as real events,
  // then follows it so the operator sees the HTTP result without leaving.
  const ping = useMutation({
    mutationFn: async (webhook: WebhookRecord) => {
      const toastId = toast.loading(`Sending test event to "${webhook.name}"…`);
      try {
        const created = (await cb.collection("_webhookDeliveries").create({
          webhookRef: webhook.id,
          event: "ping",
          collectionRef: webhook.collectionRef,
          recordId: "ping",
          payload: { event: "ping", collection: webhook.collectionRef, record: { id: "ping" } },
          attempts: 0,
          maxAttempts: 1,
          status: "pending",
          nextAttemptAt: new Date().toISOString().replace("T", " "),
        })) as unknown as { id: string };
        let delivery: WebhookDelivery | null = null;
        for (let i = 0; i < 15; i++) {
          await new Promise<void>((resolve) => setTimeout(resolve, 1000));
          delivery = (await cb.collection("_webhookDeliveries").one(created.id)) as unknown as WebhookDelivery;
          if (delivery.status === "success" || delivery.status === "failed") break;
        }
        void queryClient.invalidateQueries({ queryKey: ["webhook-deliveries"] });
        void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
        if (delivery?.status === "success") {
          toast.success(`"${webhook.name}" answered ${delivery.responseCode ?? "OK"}`, {
            id: toastId,
            description: delivery.durationMs != null ? `${delivery.durationMs} ms` : undefined,
          });
        } else if (delivery?.status === "failed") {
          toast.error(`Test event to "${webhook.name}" failed`, {
            id: toastId,
            description: delivery.error || (delivery.responseCode ? `HTTP ${delivery.responseCode}` : undefined),
          });
        } else {
          toast.message(`Test event to "${webhook.name}" is still pending`, {
            id: toastId,
            description: "Check Recent deliveries below for the result.",
          });
        }
      } catch (failure) {
        const described = describeFailure(failure);
        toast.error(`Couldn't send test event: ${described.title}`, { id: toastId, description: described.detail });
      }
    },
  });

  const webhooks = data ?? [];
  const needle = search.trim().toLowerCase();
  const visible = needle
    ? webhooks.filter((w) => `${w.name} ${w.url} ${w.collectionRef}`.toLowerCase().includes(needle))
    : webhooks;
  const collectionName = (ref: string) =>
    collections?.find((c) => c.id === ref || c.name === ref)?.name ?? ref;
  const selectedIndex = visible.findIndex((w) => w.id === selectedId);
  const selected = selectedIndex >= 0 ? (visible[selectedIndex] ?? null) : null;
  function openWebhook(id: string, nextTab = "overview") {
    setSelectedId(id);
    setTab(nextTab);
  }

  const newButton = (
    <Button size="sm" onClick={() => setDialogWebhook("new")}>
      <Plus />
      New webhook
    </Button>
  );

  const item = settingsItemFor("/settings/webhooks")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={webhooks.length > 0 ? newButton : undefined}
    >
      {error ? (
        <ErrorState title="Couldn't load webhooks" error={error} onRetry={() => void refetch()} compact />
      ) : isPending ? (
        <ListSkeleton rows={3} />
      ) : webhooks.length === 0 ? (
        <EmptyState
          compact
          icon={<Webhook />}
          title="No webhooks yet"
          description="Notify an external service with a signed POST whenever a record is created, updated or deleted."
          action={newButton}
        />
      ) : (
        <>
          <DataTableToolbar search={{ value: search, onChange: setSearch, placeholder: "Search webhooks…" }} />
          {visible.length === 0 ? (
            <EmptyState compact title="No matching webhooks" description={`Nothing matches "${search}".`} />
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border bg-card">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Webhook</TableHead>
                    <TableHead>Collection</TableHead>
                    <TableHead>Events</TableHead>
                    <TableHead>Last delivery</TableHead>
                    <TableHead className="w-40 text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {visible.map((webhook) => (
                    <TableRow
                      key={webhook.id}
                      className="cursor-pointer align-top"
                      data-state={webhook.id === selectedId ? "selected" : undefined}
                      onClick={() => openWebhook(webhook.id)}
                    >
                      <TableCell className="py-2.5">
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            className="rounded-sm text-left font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/50"
                            onClick={(e) => {
                              e.stopPropagation();
                              openWebhook(webhook.id);
                            }}
                          >
                            {webhook.name}
                          </button>
                          <StatusBadge tone={webhook.enabled ? "success" : "neutral"}>
                            {webhook.enabled ? "Enabled" : "Disabled"}
                          </StatusBadge>
                        </div>
                        <div className="mt-1 max-w-sm truncate font-mono text-xs text-muted-foreground" title={webhook.url}>
                          {webhook.url}
                        </div>
                        {!webhook.enabled && (webhook.consecutiveFailures ?? 0) > 0 ? (
                          <div className="mt-1 text-xs text-warning">
                            Stopped after <span className="font-tabular">{webhook.consecutiveFailures}</span> consecutive
                            failures
                          </div>
                        ) : null}
                      </TableCell>
                      <TableCell className="py-2.5 font-mono text-sm">{collectionName(webhook.collectionRef)}</TableCell>
                      <TableCell className="py-2.5">
                        <div className="flex flex-wrap gap-1">
                          {eventsOf(webhook).map((event) => (
                            <Badge key={event} variant="outline" className="font-mono text-2xs font-normal">
                              {event}
                            </Badge>
                          ))}
                        </div>
                      </TableCell>
                      <TableCell className="py-2.5 text-xs text-muted-foreground">
                        {webhook.lastTriggeredAt ? (
                          <>
                            <StatusBadge tone={webhook.lastStatus === "error" ? "danger" : "success"}>
                              {webhook.lastStatus === "error" ? "Failing" : "Delivered"}
                            </StatusBadge>
                            <div className="mt-1 font-mono">
                              {parseServerDate(webhook.lastTriggeredAt).toLocaleString()}
                            </div>
                            {webhook.lastMessage ? (
                              <div className="max-w-xs truncate" title={webhook.lastMessage}>
                                {webhook.lastMessage}
                              </div>
                            ) : null}
                          </>
                        ) : (
                          "Never triggered"
                        )}
                      </TableCell>
                      <TableCell className="py-2.5">
                        <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
                          <Switch
                            checked={webhook.enabled}
                            disabled={toggle.isPending && toggle.variables?.webhook.id === webhook.id}
                            aria-label={`${webhook.enabled ? "Disable" : "Enable"} ${webhook.name}`}
                            onCheckedChange={(enabled) => toggle.mutate({ webhook, enabled })}
                          />
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Send test event to ${webhook.name}`}
                            title="Send a test event"
                            disabled={ping.isPending}
                            onClick={() => ping.mutate(webhook)}
                          >
                            {ping.isPending && ping.variables?.id === webhook.id ? <Spinner /> : <Send />}
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Edit ${webhook.name}`}
                            title="Edit"
                            onClick={() => openWebhook(webhook.id, "settings")}
                          >
                            <Pencil />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Delete ${webhook.name}`}
                            title="Delete"
                            onClick={() => setDeleteTarget(webhook)}
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
        </>
      )}

      <WebhookDeliveries
        webhooks={webhooks}
        paused={dialogWebhook !== null || deleteTarget !== null || selected !== null}
      />

      {dialogWebhook ? (
        <WebhookDialog
          collections={collections ?? []}
          onOpenChange={(open) => {
            if (!open) setDialogWebhook(null);
          }}
        />
      ) : null}

      <WebhookDetailSheet
        webhook={selected}
        index={selectedIndex}
        total={visible.length}
        onStep={(delta) => {
          const next = visible[selectedIndex + delta];
          if (next) setSelectedId(next.id);
        }}
        onClose={() => setSelectedId(null)}
        tab={tab}
        onTabChange={setTab}
        collections={collections ?? []}
        collectionName={collectionName}
        onToggle={(webhook, enabled) => toggle.mutate({ webhook, enabled })}
        toggling={toggle.isPending}
        onPing={(webhook) => ping.mutate(webhook)}
        pinging={ping.isPending}
        onDelete={setDeleteTarget}
      />

      <TypedConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title="Delete webhook"
        description="Delivery to this endpoint stops immediately and its pending retries are orphaned. There is no undo."
        confirmValue={deleteTarget?.name ?? ""}
        confirmLabel="Delete webhook"
        pendingLabel="Deleting…"
        pending={remove.isPending}
        onConfirm={() => deleteTarget && remove.mutate(deleteTarget.id)}
      />
    </SettingsPage>
  );
}

/** A random 256-bit signing secret, hex-encoded. */
function generateSecret(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Client-side URL check; the server still enforces the SSRF rules. */
function urlProblem(url: string): { error?: string; warning?: string } {
  const trimmed = url.trim();
  if (!trimmed) return { error: "Enter the endpoint URL" };
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return { error: "Not a valid URL — start with https://" };
  }
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    return { error: "Only https:// (or http://) URLs are supported" };
  }
  if (parsed.protocol === "http:") {
    return { warning: "Plain http sends payloads unencrypted — use https in production." };
  }
  return {};
}

/** Create or edit one `_webhooks` record. `webhook === null` means create. */
function WebhookForm({
  webhook,
  collections,
  onDone,
  onCancel,
}: {
  webhook: WebhookRecord | null;
  collections: { id: string; name: string }[];
  onDone: () => void;
  onCancel?: () => void;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(webhook?.name ?? "");
  const [collectionRef, setCollectionRef] = useState(webhook?.collectionRef ?? "");
  const [events, setEvents] = useState<Set<string>>(
    new Set(webhook ? eventsOf(webhook) : ["create", "update", "delete"]),
  );
  const [url, setUrl] = useState(webhook?.url ?? "");
  const [secret, setSecret] = useState("");
  const [showSecret, setShowSecret] = useState(false);
  const [enabled, setEnabled] = useState(webhook?.enabled ?? true);
  const [submitted, setSubmitted] = useState(false);
  const [urlTouched, setUrlTouched] = useState(false);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const { copy, copied } = useCopyToClipboard({
    onError: () => toast.error("Couldn't copy — select the secret and copy it manually"),
  });

  const urlCheck = urlProblem(url);
  const clientErrors: Record<string, string> = {};
  if (!name.trim()) clientErrors.name = "Give the webhook a name";
  if (!collectionRef) clientErrors.collectionRef = "Pick the collection to watch";
  if (events.size === 0) clientErrors.events = "Select at least one event";
  if (urlCheck.error) clientErrors.url = urlCheck.error;
  const errorFor = (field: string, always = submitted) => fieldErrors[field] ?? (always ? clientErrors[field] : undefined);
  const urlError = fieldErrors["url"] ?? (urlTouched || submitted ? clientErrors.url : undefined);

  const save = useMutation({
    mutationFn: () => {
      const body: Record<string, unknown> = {
        name: name.trim(),
        collectionRef,
        events: EVENT_KINDS.map((k) => k.value)
          .filter((k) => events.has(k))
          .join(","),
        url: url.trim(),
        enabled,
      };
      // A blank secret box means "leave what's stored alone" on edit, and
      // "no signing" on create — same write-only convention as SMTP/S3.
      if (secret) body.secret = secret;
      return webhook ? cb.collection("_webhooks").update(webhook.id, body) : cb.collection("_webhooks").create(body);
    },
    onSuccess: () => {
      toast.success(webhook ? `Webhook "${name.trim()}" updated` : `Webhook "${name.trim()}" created`);
      void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
      setSecret("");
      onDone();
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(`Couldn't save webhook: ${described.title}`, { description: described.detail });
      }
    },
  });

  function toggleEvent(kind: string, checked: boolean) {
    setFieldErrors(({ events: _events, ...rest }) => rest);
    setEvents((prev) => {
      const next = new Set(prev);
      if (checked) next.add(kind);
      else next.delete(kind);
      return next;
    });
  }

  function submit() {
    setSubmitted(true);
    if (Object.keys(clientErrors).length > 0) return;
    save.mutate();
  }

  const fieldError = (id: string, message: string | undefined) =>
    message ? (
      <p id={id} role="alert" className="text-xs text-destructive">
        {message}
      </p>
    ) : null;

  const nameError = errorFor("name");
  const collectionError = errorFor("collectionRef");
  const eventsError = errorFor("events");
  const secretError = fieldErrors["secret"];

  return (
    <form
      className="flex flex-col gap-4"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-name">Name</Label>
            <Input
              id="webhook-name"
              value={name}
              autoFocus={!webhook}
              onChange={(e) => {
                setName(e.target.value);
                setFieldErrors(({ name: _n, ...rest }) => rest);
              }}
              placeholder="Slack notify"
              aria-invalid={nameError ? true : undefined}
              aria-describedby={nameError ? "webhook-name-error" : undefined}
            />
            {fieldError("webhook-name-error", nameError)}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-collection">Collection</Label>
            <Select
              value={collectionRef}
              onValueChange={(value) => {
                setCollectionRef(value);
                setFieldErrors(({ collectionRef: _c, ...rest }) => rest);
              }}
            >
              <SelectTrigger
                id="webhook-collection"
                className="w-full font-mono"
                aria-invalid={collectionError ? true : undefined}
                aria-describedby={collectionError ? "webhook-collection-error" : undefined}
              >
                <SelectValue placeholder="Select a collection…" />
              </SelectTrigger>
              <SelectContent>
                {collections.map((c) => (
                  <SelectItem key={c.id} value={c.name} className="font-mono">
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {fieldError("webhook-collection-error", collectionError)}
          </div>

          <fieldset
            className="flex flex-col gap-1.5"
            aria-describedby={eventsError ? "webhook-events-error" : "webhook-events-help"}
          >
            <legend className="mb-1.5 text-sm font-medium">Events</legend>
            <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-3">
              {EVENT_KINDS.map((kind) => (
                <label
                  key={kind.value}
                  htmlFor={`webhook-event-${kind.value}`}
                  className={cn(
                    "flex cursor-pointer items-start gap-2 rounded-lg border px-2.5 py-2 transition-colors duration-fast ease-out-strong",
                    events.has(kind.value) ? "border-border-strong bg-surface-sunken" : "border-border hover:bg-surface-sunken",
                  )}
                >
                  <Checkbox
                    id={`webhook-event-${kind.value}`}
                    className="mt-0.5"
                    checked={events.has(kind.value)}
                    onCheckedChange={(checked) => toggleEvent(kind.value, checked === true)}
                  />
                  <span className="flex flex-col">
                    <span className="text-sm font-medium">{kind.label}</span>
                    <span className="text-xs text-muted-foreground">{kind.help}</span>
                  </span>
                </label>
              ))}
            </div>
            {eventsError ? (
              fieldError("webhook-events-error", eventsError)
            ) : (
              <p id="webhook-events-help" className="text-xs text-muted-foreground">
                Choose which record events trigger a delivery.
              </p>
            )}
          </fieldset>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-url">Endpoint URL</Label>
            <Input
              id="webhook-url"
              type="url"
              inputMode="url"
              autoComplete="off"
              spellCheck={false}
              className="font-mono"
              value={url}
              onChange={(e) => {
                setUrl(e.target.value);
                setFieldErrors(({ url: _u, ...rest }) => rest);
              }}
              onBlur={() => setUrlTouched(true)}
              placeholder="https://example.com/hooks/cratebase"
              aria-invalid={urlError ? true : undefined}
              aria-describedby="webhook-url-note"
            />
            {urlError ? (
              fieldError("webhook-url-note", urlError)
            ) : (
              <p id="webhook-url-note" className={cn("text-xs", urlCheck.warning ? "text-warning" : "text-muted-foreground")}>
                {urlCheck.warning ?? "Receives a JSON POST. Private and loopback addresses are blocked by the server."}
              </p>
            )}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="webhook-secret">Signing secret</Label>
            <div className="flex gap-1.5">
              <Input
                id="webhook-secret"
                type={showSecret ? "text" : "password"}
                autoComplete="new-password"
                spellCheck={false}
                className="font-mono"
                value={secret}
                onChange={(e) => {
                  setSecret(e.target.value);
                  setFieldErrors(({ secret: _s, ...rest }) => rest);
                }}
                placeholder={webhook ? "Stored — leave blank to keep" : "Optional"}
                aria-invalid={secretError ? true : undefined}
                aria-describedby="webhook-secret-note"
              />
              <Button
                type="button"
                variant="outline"
                size="icon"
                aria-label={showSecret ? "Hide secret" : "Show secret"}
                aria-pressed={showSecret}
                onClick={() => setShowSecret((v) => !v)}
              >
                {showSecret ? <EyeOff /> : <Eye />}
              </Button>
              <Button
                type="button"
                variant="outline"
                size="icon"
                aria-label="Copy secret"
                disabled={!secret}
                onClick={() => void copy(secret)}
              >
                {copied ? <Check className="text-success" /> : <Copy />}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setSecret(generateSecret());
                  setShowSecret(true);
                }}
              >
                <Sparkles />
                Generate
              </Button>
            </div>
            {secretError ? (
              fieldError("webhook-secret-note", secretError)
            ) : (
              <p id="webhook-secret-note" className="text-xs text-muted-foreground">
                {webhook
                  ? "The stored secret is never shown again. Enter a new one to rotate it."
                  : "Signs each request with an HMAC-SHA256 X-Cratebase-Signature header. Copy it now — it can't be read back later."}
              </p>
            )}
          </div>

          <div className="flex items-center gap-2">
            <Switch id="webhook-enabled" checked={enabled} onCheckedChange={setEnabled} />
            <Label htmlFor="webhook-enabled">Enabled</Label>
          </div>

          <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
            {onCancel ? (
              <Button type="button" variant="outline" onClick={onCancel}>
                Cancel
              </Button>
            ) : null}
            <Button type="submit" disabled={save.isPending}>
              {save.isPending ? <Spinner /> : null}
              {webhook ? "Save changes" : "Create webhook"}
            </Button>
          </div>
    </form>
  );
}

/** Create dialog for a new `_webhooks` record; existing rows are edited in
 * the detail sheet's Settings tab. */
function WebhookDialog({
  collections,
  onOpenChange,
}: {
  collections: { id: string; name: string }[];
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>New webhook</DialogTitle>
          <DialogDescription>
            Fires after the record write already succeeded — it never blocks the write, and failed deliveries are retried
            with backoff.
          </DialogDescription>
        </DialogHeader>
        <WebhookForm
          webhook={null}
          collections={collections}
          onDone={() => onOpenChange(false)}
          onCancel={() => onOpenChange(false)}
        />
      </DialogContent>
    </Dialog>
  );
}

const DELIVERY_STATUSES = ["pending", "in_progress", "success", "failed"] as const;
type DeliveryStatus = (typeof DELIVERY_STATUSES)[number];

const DELIVERY_STATUS_VIEW: Record<DeliveryStatus, { label: string; tone: Tone }> = {
  pending: { label: "Pending", tone: "warning" },
  in_progress: { label: "Sending", tone: "info" },
  success: { label: "Delivered", tone: "success" },
  failed: { label: "Failed", tone: "danger" },
};

/** A `_webhookDeliveries` record — see
 * `crates/server/src/webhook_deliveries.rs`. */
interface WebhookDelivery {
  id: string;
  webhookRef: string;
  event: string;
  status: DeliveryStatus;
  attempts: number;
  maxAttempts: number;
  responseCode?: string;
  responseBody?: string;
  durationMs?: number | null;
  nextAttemptAt?: string;
  deliveredAt?: string;
  error?: string;
  payload?: unknown;
  created: string;
}

const DELIVERIES_PAGE_SIZE = 50;

/** Recent `_webhookDeliveries` rows across every webhook, with status and
 * webhook filters and a Retry button — every dispatch is durably retried by
 * an always-on worker (`crates/server/src/webhook_deliveries.rs`), not
 * fired-and-forgotten, so this is the operational view into that. Polls
 * while visible, so a retry or test event shows its result on its own. */
function WebhookDeliveries({ webhooks, paused }: { webhooks: WebhookRecord[]; paused: boolean }) {
  const [statusFilter, setStatusFilter] = useState<DeliveryStatus | "all">("all");
  const [webhookFilter, setWebhookFilter] = useState<string>("all");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [live, setLive] = useState(true);

  const { data, isPending, error, refetch, isFetching } = useQuery({
    queryKey: ["webhook-deliveries", statusFilter, webhookFilter],
    queryFn: () => {
      const clauses: string[] = [];
      if (statusFilter !== "all") clauses.push(`status = "${statusFilter}"`);
      if (webhookFilter !== "all") clauses.push(`webhookRef = "${webhookFilter}"`);
      return cb.collection("_webhookDeliveries").list({
        page: 1,
        perPage: DELIVERIES_PAGE_SIZE,
        sort: "-created",
        filter: clauses.length ? clauses.join(" && ") : undefined,
      }) as unknown as Promise<{ items: WebhookDelivery[] }>;
    },
    refetchInterval: live && !paused ? LIVE_INTERVAL_MS : false,
  });

  const replay = useReplayDelivery();

  const deliveries = data?.items ?? [];
  const nameFor = (webhookRef: string) => webhooks.find((w) => w.id === webhookRef)?.name ?? webhookRef;
  const filtered = statusFilter !== "all" || webhookFilter !== "all";

  return (
    <section className="flex flex-col gap-3" aria-labelledby="webhook-deliveries-title">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <div className="flex flex-col">
          <h3 id="webhook-deliveries-title" className="text-sm font-medium">
            Recent deliveries
          </h3>
          <p className="max-w-measure text-sm text-muted-foreground">
            Every dispatch is durably retried with backoff. Retry resends one immediately.
          </p>
        </div>
        <LiveToggle live={live && !paused} onLiveChange={setLive} fetching={isFetching} onRefresh={() => void refetch()} />
      </div>

      <DataTableToolbar
        filters={
          <>
            <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as DeliveryStatus | "all")}>
              <SelectTrigger size="sm" className="w-36" aria-label="Filter by status">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All statuses</SelectItem>
                {DELIVERY_STATUSES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {DELIVERY_STATUS_VIEW[s].label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select value={webhookFilter} onValueChange={setWebhookFilter}>
              <SelectTrigger size="sm" className="w-44" aria-label="Filter by webhook">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All webhooks</SelectItem>
                {webhooks.map((w) => (
                  <SelectItem key={w.id} value={w.id}>
                    {w.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </>
        }
      />

      {error ? (
        <ErrorState title="Couldn't load deliveries" error={error} onRetry={() => void refetch()} compact />
      ) : isPending ? (
        <ListSkeleton rows={4} />
      ) : deliveries.length === 0 ? (
        <EmptyState
          compact
          icon={<Webhook />}
          title={filtered ? "No deliveries match these filters" : "No deliveries yet"}
          description={
            filtered
              ? "Clear a filter to see more."
              : "Send a test event from a webhook above, or change a record in a watched collection."
          }
        />
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-8" />
                <TableHead>Time</TableHead>
                <TableHead>Webhook</TableHead>
                <TableHead>Event</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>HTTP</TableHead>
                <TableHead className="text-right">Duration</TableHead>
                <TableHead className="text-right">Attempts</TableHead>
                <TableHead className="w-20 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {deliveries.map((delivery) => {
                const view = DELIVERY_STATUS_VIEW[delivery.status] ?? {
                  label: String(delivery.status),
                  tone: "neutral" as Tone,
                };
                const open = expanded === delivery.id;
                const retrying = replay.isPending && replay.variables === delivery.id;
                return (
                  <Fragment key={delivery.id}>
                    <TableRow className="align-top">
                      <TableCell className="py-1.5">
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          aria-expanded={open}
                          aria-label={`${open ? "Hide" : "Show"} details for delivery to ${nameFor(delivery.webhookRef)}`}
                          onClick={() => setExpanded(open ? null : delivery.id)}
                        >
                          <ChevronRight
                            className={cn("transition-transform duration-fast ease-out-strong", open && "rotate-90")}
                          />
                        </Button>
                      </TableCell>
                      <TableCell className="py-1.5 font-mono text-xs whitespace-nowrap text-muted-foreground">
                        {parseServerDate(delivery.created).toLocaleString()}
                      </TableCell>
                      <TableCell className="py-1.5 font-medium">{nameFor(delivery.webhookRef)}</TableCell>
                      <TableCell className="py-1.5 font-mono text-xs">{delivery.event}</TableCell>
                      <TableCell className="py-1.5">
                        <StatusBadge tone={view.tone}>{view.label}</StatusBadge>
                      </TableCell>
                      <TableCell className="py-1.5 font-mono text-xs">{delivery.responseCode || "—"}</TableCell>
                      <TableCell className="py-1.5 text-right font-tabular text-xs">
                        {delivery.durationMs != null ? `${delivery.durationMs} ms` : "—"}
                      </TableCell>
                      <TableCell className="py-1.5 text-right font-tabular text-xs">
                        {delivery.attempts} / {delivery.maxAttempts}
                      </TableCell>
                      <TableCell className="py-1.5 text-right">
                        <Button
                          variant="ghost"
                          size="xs"
                          aria-label={`Retry delivery to ${nameFor(delivery.webhookRef)}`}
                          disabled={retrying || delivery.status === "pending" || delivery.status === "in_progress"}
                          onClick={() => replay.mutate(delivery.id)}
                        >
                          {retrying ? <Spinner /> : "Retry"}
                        </Button>
                      </TableCell>
                    </TableRow>
                    {open ? (
                      <TableRow className="bg-surface-sunken hover:bg-surface-sunken">
                        <TableCell />
                        <TableCell colSpan={8} className="py-3 whitespace-normal">
                          <dl className="grid grid-cols-1 gap-x-6 gap-y-2 text-xs sm:grid-cols-[auto_1fr]">
                            <dt className="text-muted-foreground">Delivery ID</dt>
                            <dd className="font-mono">{delivery.id}</dd>
                            {delivery.nextAttemptAt && delivery.status === "pending" ? (
                              <>
                                <dt className="text-muted-foreground">Next attempt</dt>
                                <dd className="font-mono">{parseServerDate(delivery.nextAttemptAt).toLocaleString()}</dd>
                              </>
                            ) : null}
                            {delivery.deliveredAt ? (
                              <>
                                <dt className="text-muted-foreground">Delivered at</dt>
                                <dd className="font-mono">{parseServerDate(delivery.deliveredAt).toLocaleString()}</dd>
                              </>
                            ) : null}
                            {delivery.error ? (
                              <>
                                <dt className="text-muted-foreground">Error</dt>
                                <dd className="font-mono break-words text-destructive">{delivery.error}</dd>
                              </>
                            ) : null}
                            <dt className="text-muted-foreground">Response</dt>
                            <dd>
                              {delivery.responseBody ? (
                                <pre className="max-h-40 overflow-auto rounded-md border border-border bg-card p-2 font-mono text-xs break-words whitespace-pre-wrap">
                                  {delivery.responseBody}
                                </pre>
                              ) : (
                                <span className="text-muted-foreground">No response body captured</span>
                              )}
                            </dd>
                          </dl>
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </Fragment>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  );
}

/** Queues one delivery for an immediate retry. */
function useReplayDelivery() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      cb.send<void>(`/api/webhooks/deliveries/${encodeURIComponent(id)}/replay`, { method: "POST" }),
    onSuccess: () => {
      toast.success("Delivery queued for retry");
      void queryClient.invalidateQueries({ queryKey: ["webhook-deliveries"] });
    },
    onError: failureToast,
  });
}
/** One delivery inside the webhook sheet: a summary row that expands to the
 * full attempt record (error, response, payload) with a Retry button. */
function DeliveryItem({
  delivery,
  replay,
}: {
  delivery: WebhookDelivery;
  replay: UseMutationResult<void, unknown, string>;
}) {
  const [open, setOpen] = useState(false);
  const view = DELIVERY_STATUS_VIEW[delivery.status] ?? { label: String(delivery.status), tone: "neutral" as Tone };
  const retrying = replay.isPending && replay.variables === delivery.id;
  const busy = delivery.status === "pending" || delivery.status === "in_progress";
  return (
    <li className="border-b border-border last:border-b-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 py-2 text-left text-sm outline-none transition-colors duration-fast ease-out-strong hover:bg-surface-sunken focus-visible:ring-2 focus-visible:ring-ring/50"
      >
        <ChevronRight
          className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform duration-fast ease-out-strong", open && "rotate-90")}
        />
        <StatusBadge tone={view.tone}>{view.label}</StatusBadge>
        <span className="font-mono text-xs">{delivery.event}</span>
        <span className="font-tabular font-mono text-xs text-muted-foreground">{delivery.responseCode || "—"}</span>
        <span className="font-tabular ml-auto shrink-0 text-xs text-muted-foreground">
          {delivery.durationMs != null ? `${delivery.durationMs} ms · ` : ""}
          <TimeAgo value={delivery.created} />
        </span>
      </button>
      {open ? (
        <div className="flex flex-col gap-3 pr-1 pb-3 pl-5">
          <DetailList>
            <DetailRow label="Delivery id">
              <span className="font-mono text-xs">{delivery.id}</span>
            </DetailRow>
            <DetailRow label="Attempts">
              <span className="font-tabular">
                {delivery.attempts} / {delivery.maxAttempts}
              </span>
            </DetailRow>
            <DetailRow label="HTTP">
              <span className="font-tabular font-mono">{delivery.responseCode || "—"}</span>
            </DetailRow>
            <DetailRow label="Duration">
              <span className="font-tabular">{delivery.durationMs != null ? `${delivery.durationMs} ms` : "—"}</span>
            </DetailRow>
            <DetailRow label="Created">
              <TimeAgo value={delivery.created} />
            </DetailRow>
            {delivery.nextAttemptAt && delivery.status === "pending" ? (
              <DetailRow label="Next attempt">
                <TimeAgo value={delivery.nextAttemptAt} />
              </DetailRow>
            ) : null}
            {delivery.deliveredAt ? (
              <DetailRow label="Delivered">
                <TimeAgo value={delivery.deliveredAt} />
              </DetailRow>
            ) : null}
          </DetailList>
          {delivery.error ? (
            <DetailCallout title="Delivery error">
              <span className="font-mono break-words">{delivery.error}</span>
            </DetailCallout>
          ) : null}
          {delivery.responseBody ? (
            <CodeBlock title="Response" plain={delivery.responseBody} copyLabel="Copy response" />
          ) : (
            <p className="m-0 text-sm text-muted-foreground">No response body captured</p>
          )}
          {delivery.payload !== undefined && delivery.payload !== null && delivery.payload !== "" ? (
            <CodeBlock
              title="Request payload"
              json={typeof delivery.payload === "string" ? delivery.payload : JSON.stringify(delivery.payload, null, 2)}
              copyLabel="Copy payload"
            />
          ) : null}
          <div>
            <Button
              size="sm"
              variant="outline"
              aria-label="Retry delivery"
              disabled={retrying || busy}
              onClick={() => replay.mutate(delivery.id)}
            >
              {retrying ? <Spinner /> : <RotateCw />}
              Retry
            </Button>
          </div>
        </div>
      ) : null}
    </li>
  );
}

/** Everything about one webhook: overview, its deliveries, and the edit
 * form. The pager walks the table's (filtered) webhooks. */
function WebhookDetailSheet({
  webhook,
  index,
  total,
  onStep,
  onClose,
  tab,
  onTabChange,
  collections,
  collectionName,
  onToggle,
  toggling,
  onPing,
  pinging,
  onDelete,
}: {
  webhook: WebhookRecord | null;
  index: number;
  total: number;
  onStep: (delta: -1 | 1) => void;
  onClose: () => void;
  tab: string;
  onTabChange: (tab: string) => void;
  collections: { id: string; name: string }[];
  collectionName: (ref: string) => string;
  onToggle: (webhook: WebhookRecord, enabled: boolean) => void;
  toggling: boolean;
  onPing: (webhook: WebhookRecord) => void;
  pinging: boolean;
  onDelete: (webhook: WebhookRecord) => void;
}) {
  const replay = useReplayDelivery();
  const id = webhook?.id;
  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["webhook-deliveries", "webhook", id],
    enabled: id !== undefined,
    queryFn: () =>
      cb.collection("_webhookDeliveries").list({
        page: 1,
        perPage: DELIVERIES_PAGE_SIZE,
        sort: "-created",
        filter: `webhookRef = "${id}"`,
      }) as unknown as Promise<{ items: WebhookDelivery[]; totalItems?: number }>,
    refetchInterval: LIVE_INTERVAL_MS,
  });
  const deliveries = data?.items ?? [];
  const deliveryCount = data?.totalItems ?? deliveries.length;

  return (
    <DetailSheet open={webhook !== null} onOpenChange={(open) => !open && onClose()}>
      {webhook ? (
        <>
          <DetailSheetHeader
            id={webhook.id}
            pager={{
              index: index + 1,
              total,
              hasPrev: index > 0,
              hasNext: index < total - 1,
              onPrev: () => onStep(-1),
              onNext: () => onStep(1),
            }}
            menu={[
              { label: "Delete webhook", destructive: true, icon: <Trash2 />, onSelect: () => onDelete(webhook) },
            ]}
            title={webhook.name}
            description={`Webhook ${webhook.name} posting to ${webhook.url}`}
            status={
              <StatusBadge
                tone={!webhook.enabled ? "neutral" : webhook.lastStatus === "error" ? "danger" : "success"}
              >
                {!webhook.enabled ? "Disabled" : webhook.lastStatus === "error" ? "Failing" : "Enabled"}
              </StatusBadge>
            }
            meta={[
              <span key="events" className="font-tabular">
                {eventsOf(webhook).length} {eventsOf(webhook).length === 1 ? "event" : "events"}
              </span>,
              <span key="collection" className="font-mono">{collectionName(webhook.collectionRef)}</span>,
              webhook.lastTriggeredAt ? (
                <Fragment key="fired">
                  Last fired <TimeAgo value={webhook.lastTriggeredAt} />
                </Fragment>
              ) : (
                "Never triggered"
              ),
            ]}
            actions={
              <>
                <Button size="sm" disabled={pinging} onClick={() => onPing(webhook)}>
                  {pinging ? <Spinner /> : <Send />}
                  Send test
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={toggling}
                  onClick={() => onToggle(webhook, !webhook.enabled)}
                >
                  {webhook.enabled ? <Pause /> : <Play />}
                  {webhook.enabled ? "Disable" : "Enable"}
                </Button>
              </>
            }
          />
          <DetailTabs
            value={tab}
            onValueChange={onTabChange}
            tabs={[
              { value: "overview", label: "Overview" },
              { value: "deliveries", label: "Deliveries", count: deliveryCount },
              { value: "settings", label: "Settings" },
            ]}
          >
            <DetailTabPanel value="overview">
              <DetailBody>
                {webhook.lastStatus === "error" ? (
                  <DetailCallout
                    title="Last delivery failed"
                    action={
                      <Button size="xs" variant="outline" onClick={() => onTabChange("deliveries")}>
                        View deliveries
                      </Button>
                    }
                  >
                    <span className="break-words">{webhook.lastMessage || "No error message recorded."}</span>
                    {webhook.lastTriggeredAt ? (
                      <>
                        {" "}
                        <TimeAgo value={webhook.lastTriggeredAt} />
                      </>
                    ) : null}
                  </DetailCallout>
                ) : null}
                {!webhook.enabled && (webhook.consecutiveFailures ?? 0) > 0 ? (
                  <DetailCallout tone="warning" title="Stopped automatically">
                    Disabled after <span className="font-tabular">{webhook.consecutiveFailures}</span> consecutive
                    failures. Fix the endpoint, then enable it again.
                  </DetailCallout>
                ) : null}
                <DetailSection title="Details">
                  <DetailList>
                    <DetailRow label="Endpoint">
                      <span className="font-mono text-xs break-all">{webhook.url}</span>
                    </DetailRow>
                    <DetailRow label="Collection">
                      <span className="font-mono">{collectionName(webhook.collectionRef)}</span>
                    </DetailRow>
                    <DetailRow label="Events">
                      <span className="flex flex-wrap gap-1">
                        {eventsOf(webhook).map((event) => (
                          <DetailChip key={event}>{event}</DetailChip>
                        ))}
                      </span>
                    </DetailRow>
                    <DetailRow label="Status">
                      {webhook.enabled ? "Enabled" : "Disabled"}
                    </DetailRow>
                    <DetailRow label="Signing" hint="The secret is write-only — rotate it in Settings.">
                      HMAC-SHA256
                    </DetailRow>
                  </DetailList>
                </DetailSection>
                <DetailSection title="Last delivery">
                  <DetailList>
                    <DetailRow label="Result">
                      {webhook.lastTriggeredAt ? (
                        <StatusBadge tone={webhook.lastStatus === "error" ? "danger" : "success"}>
                          {webhook.lastStatus === "error" ? "Failing" : "Delivered"}
                        </StatusBadge>
                      ) : (
                        <span className="text-muted-foreground">Never triggered</span>
                      )}
                    </DetailRow>
                    <DetailRow label="Triggered">
                      <TimeAgo value={webhook.lastTriggeredAt} fallback="Never" />
                    </DetailRow>
                    {webhook.lastMessage ? (
                      <DetailRow label="Message">
                        <span className="font-mono text-xs break-words">{webhook.lastMessage}</span>
                      </DetailRow>
                    ) : null}
                    <DetailRow label="Failures in a row">
                      <span className="font-tabular">{webhook.consecutiveFailures ?? 0}</span>
                    </DetailRow>
                  </DetailList>
                </DetailSection>
              </DetailBody>
            </DetailTabPanel>
            <DetailTabPanel value="deliveries">
              <DetailBody>
                {error ? (
                  <ErrorState title="Couldn't load deliveries" error={error} onRetry={() => void refetch()} compact />
                ) : isPending ? (
                  <ListSkeleton rows={4} />
                ) : deliveries.length === 0 ? (
                  <DetailEmpty
                    icon={<Webhook />}
                    title="No deliveries yet"
                    description="Send a test event, or change a record in the watched collection."
                  />
                ) : (
                  <ul className="m-0 flex list-none flex-col p-0">
                    {deliveries.map((delivery) => (
                      <DeliveryItem key={delivery.id} delivery={delivery} replay={replay} />
                    ))}
                  </ul>
                )}
              </DetailBody>
            </DetailTabPanel>
            <DetailTabPanel value="settings">
              <DetailBody>
                <WebhookForm key={webhook.id} webhook={webhook} collections={collections} onDone={() => undefined} />
              </DetailBody>
            </DetailTabPanel>
          </DetailTabs>
        </>
      ) : null}
    </DetailSheet>
  );
}
