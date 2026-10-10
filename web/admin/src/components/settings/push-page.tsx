import { useMemo, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { Plus, Send, Smartphone, Trash2 } from "lucide-react";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatusBadge } from "@/components/layout/stat-card";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import {
  SecretSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

type Push = ServerSettings["push"];
type PushTrigger = Push["triggers"][number];

type Draft = Pick<ServerSettings, "push">;

const TRIGGER_EVENTS = ["create", "update", "delete"] as const;
const STORED_HINT = "•••••••• (unchanged)";

function draftOf(settings: ServerSettings): Draft {
  return {
    push: {
      vapid: { ...settings.push.vapid, privateKey: "" },
      fcm: { ...settings.push.fcm, serviceAccountJson: "" },
      apns: { ...settings.push.apns, key: "" },
      triggers: settings.push.triggers.map((t) => ({ ...t })),
    },
  };
}

/** Only send a secret when one was typed — an empty box means "keep what
 * is stored", not "clear it". */
function payloadOf(draft: Draft) {
  const vapid: Record<string, unknown> = { ...draft.push.vapid };
  if (!draft.push.vapid.privateKey) delete vapid.privateKey;
  const fcm: Record<string, unknown> = { ...draft.push.fcm };
  if (!draft.push.fcm.serviceAccountJson) delete fcm.serviceAccountJson;
  const apns: Record<string, unknown> = { ...draft.push.apns };
  if (!draft.push.apns.key) delete apns.key;
  return { push: { vapid, fcm, apns, triggers: draft.push.triggers } };
}

/** Field-keyed problems for the inline messages, plus the flat list the save
 * bar shows. Only enabled providers/triggers are checked. */
function problemsOf(draft: Draft): Record<string, string> {
  const problems: Record<string, string> = {};
  const { vapid, apns, triggers } = draft.push;
  if (vapid.enabled) {
    if (!vapid.publicKey.trim()) problems["vapid-public"] = "Web Push needs a public key";
    if (!vapid.subject.trim()) problems["vapid-subject"] = "Web Push needs a subject";
  }
  if (apns.enabled) {
    if (!apns.keyId.trim()) problems["apns-keyid"] = "APNs needs a key ID";
    if (!apns.teamId.trim()) problems["apns-teamid"] = "APNs needs a team ID";
    if (!apns.bundleId.trim()) problems["apns-bundleid"] = "APNs needs a bundle ID";
  }
  triggers.forEach((trigger, i) => {
    if (!trigger.enabled) return;
    if (!trigger.collection.trim()) problems[`trigger-${i}-collection`] = `Trigger ${i + 1} needs a collection`;
    if (!trigger.events.trim()) problems[`trigger-${i}-events`] = `Trigger ${i + 1} needs at least one event`;
  });
  return problems;
}

function emptyTrigger(): PushTrigger {
  return { enabled: true, collection: "", events: "", title: "", body: "", targetField: "" };
}

function eventsOf(trigger: PushTrigger): string[] {
  return trigger.events
    .split(",")
    .map((e) => e.trim())
    .filter(Boolean);
}

function isDirty(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

/** `/settings/integrations?tab=push` — provider credentials (write-only
 * secrets), record-event triggers, and a device list with test-send. */
export function PushPage() {
  const { data: settings, isPending } = useSettings();
  const { data: collections } = useCollections();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(undefined);

  if (settings && (draft === null || (seedKey !== settings && !isDirty(draft, settings)))) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  const problems = useMemo(() => (draft ? problemsOf(draft) : {}), [draft]);
  const errors = Object.values(problems);

  if (isPending || !draft || !settings) {
    return (
      <div className="flex w-full max-w-3xl flex-col gap-4" aria-busy="true" aria-label="Loading push settings">
        <Skeleton className="h-48 w-full" />
        <Skeleton className="h-48 w-full" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  function patch(next: Partial<Push>) {
    setDraft((d) => (d ? { push: { ...d.push, ...next } } : d));
  }

  function patchTrigger(index: number, next: Partial<PushTrigger>) {
    if (!draft) return;
    const triggers = draft.push.triggers.map((t, i) => (i === index ? { ...t, ...next } : t));
    patch({ triggers });
  }

  function toggleTriggerEvent(index: number, trigger: PushTrigger, event: string, checked: boolean) {
    const next = new Set(eventsOf(trigger));
    if (checked) next.add(event);
    else next.delete(event);
    patchTrigger(index, { events: TRIGGER_EVENTS.filter((e) => next.has(e)).join(",") });
  }

  function submit() {
    if (!draft || errors.length > 0) return;
    save.mutate(payloadOf(draft), {
      onSuccess: () => {
        toast.success("Push settings saved");
        // Secrets were consumed; clear the boxes so they read as "stored".
        setDraft((d) =>
          d
            ? {
                push: {
                  ...d.push,
                  vapid: { ...d.push.vapid, privateKey: "" },
                  fcm: { ...d.push.fcm, serviceAccountJson: "" },
                  apns: { ...d.push.apns, key: "" },
                },
              }
            : d,
        );
      },
      onError: (error) => {
        const failure = describeFailure(error);
        const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
        toast.error(`Couldn't save push settings: ${failure.title}`, {
          description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
        });
      },
    });
  }

  const { vapid, fcm, apns, triggers } = draft.push;
  const dirty = isDirty(draft, settings);
  const providerBadge = (enabled: boolean) => (
    <StatusBadge tone={enabled ? "success" : "neutral"}>{enabled ? "Enabled" : "Off"}</StatusBadge>
  );
  const collectionNames = (collections ?? []).map((c) => c.name);

  const item = settingsItemFor("/settings/push")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="form">
      <SettingsSection
        title="Web Push (VAPID)"
        description="Delivers to browsers subscribed with the Push API. No per-platform app registration needed."
        action={providerBadge(vapid.enabled)}
      >
        <SettingRow label="Enabled" htmlFor="push-vapid-enabled">
          <ToggleSetting
            id="push-vapid-enabled"
            checked={vapid.enabled}
            onChange={(enabled) => patch({ vapid: { ...vapid, enabled } })}
            label={vapid.enabled ? "Sending web push" : "Not sending web push"}
          />
        </SettingRow>
        <SettingRow
          label="Public key"
          htmlFor="push-vapid-public"
          help="The application server key your web app passes to pushManager.subscribe()."
          error={problems["vapid-public"]}
        >
          <TextSetting
            id="push-vapid-public"
            value={vapid.publicKey}
            onChange={(publicKey) => patch({ vapid: { ...vapid, publicKey } })}
            invalid={Boolean(problems["vapid-public"])}
            mono
          />
        </SettingRow>
        <SettingRow
          label="Private key"
          htmlFor="push-vapid-private"
          help="Write-only — never sent back. Leave blank to keep the stored one."
        >
          <SecretSetting
            id="push-vapid-private"
            value={vapid.privateKey ?? ""}
            onChange={(privateKey) => patch({ vapid: { ...vapid, privateKey } })}
            storedHint={STORED_HINT}
          />
        </SettingRow>
        <SettingRow
          label="Subject"
          htmlFor="push-vapid-subject"
          help="A contact URI push services can use to reach the operator, e.g. mailto:ops@example.com."
          error={problems["vapid-subject"]}
        >
          <TextSetting
            id="push-vapid-subject"
            value={vapid.subject}
            onChange={(subject) => patch({ vapid: { ...vapid, subject } })}
            placeholder="mailto:ops@example.com"
            invalid={Boolean(problems["vapid-subject"])}
            mono
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Firebase Cloud Messaging (Android)"
        description="Delivers to Android devices via the FCM HTTP v1 API."
        action={providerBadge(fcm.enabled)}
      >
        <SettingRow label="Enabled" htmlFor="push-fcm-enabled">
          <ToggleSetting
            id="push-fcm-enabled"
            checked={fcm.enabled}
            onChange={(enabled) => patch({ fcm: { ...fcm, enabled } })}
            label={fcm.enabled ? "Sending FCM push" : "Not sending FCM push"}
          />
        </SettingRow>
        <SettingRow
          label="Service account JSON"
          htmlFor="push-fcm-sa"
          help="Write-only — never sent back. Paste the full contents of the Firebase service-account key file; leave blank to keep the stored one."
        >
          <SecretTextarea
            id="push-fcm-sa"
            value={fcm.serviceAccountJson ?? ""}
            onChange={(serviceAccountJson) => patch({ fcm: { ...fcm, serviceAccountJson } })}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Apple Push Notifications (iOS)"
        description="Delivers to iOS devices via the APNs HTTP/2 provider API, using token-based (.p8) auth."
        action={providerBadge(apns.enabled)}
      >
        <SettingRow label="Enabled" htmlFor="push-apns-enabled">
          <ToggleSetting
            id="push-apns-enabled"
            checked={apns.enabled}
            onChange={(enabled) => patch({ apns: { ...apns, enabled } })}
            label={apns.enabled ? "Sending APNs push" : "Not sending APNs push"}
          />
        </SettingRow>
        <SettingRow
          label="Key (.p8)"
          htmlFor="push-apns-key"
          help="Write-only — never sent back. Paste the full .p8 file; leave blank to keep the stored one."
        >
          <SecretTextarea
            id="push-apns-key"
            value={apns.key ?? ""}
            onChange={(key) => patch({ apns: { ...apns, key } })}
          />
        </SettingRow>
        <SettingRow
          label="Key ID"
          htmlFor="push-apns-keyid"
          help="The 10-character ID shown next to the key in your Apple Developer account."
          error={problems["apns-keyid"]}
        >
          <TextSetting
            id="push-apns-keyid"
            value={apns.keyId}
            onChange={(keyId) => patch({ apns: { ...apns, keyId } })}
            invalid={Boolean(problems["apns-keyid"])}
            mono
          />
        </SettingRow>
        <SettingRow label="Team ID" htmlFor="push-apns-teamid" error={problems["apns-teamid"]}>
          <TextSetting
            id="push-apns-teamid"
            value={apns.teamId}
            onChange={(teamId) => patch({ apns: { ...apns, teamId } })}
            invalid={Boolean(problems["apns-teamid"])}
            mono
          />
        </SettingRow>
        <SettingRow
          label="Bundle ID"
          htmlFor="push-apns-bundleid"
          help="Your app's bundle identifier — used as the APNs topic."
          error={problems["apns-bundleid"]}
        >
          <TextSetting
            id="push-apns-bundleid"
            value={apns.bundleId}
            onChange={(bundleId) => patch({ apns: { ...apns, bundleId } })}
            placeholder="com.example.app"
            invalid={Boolean(problems["apns-bundleid"])}
            mono
          />
        </SettingRow>
        <SettingRow
          label="Production"
          htmlFor="push-apns-production"
          help="Off uses the sandbox APNs endpoint for development-signed builds."
        >
          <ToggleSetting
            id="push-apns-production"
            checked={apns.production}
            onChange={(production) => patch({ apns: { ...apns, production } })}
            label={apns.production ? "api.push.apple.com" : "api.sandbox.push.apple.com"}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Triggers"
        description="Record-event → push rules. Each enabled trigger pushes when a matching event happens on its collection."
        action={
          <Button type="button" variant="outline" size="sm" onClick={() => patch({ triggers: [...triggers, emptyTrigger()] })}>
            <Plus />
            Add trigger
          </Button>
        }
      >
        {triggers.length === 0 ? (
          <EmptyState
            compact
            title="No triggers"
            description="Records never push automatically. Add a trigger to notify devices when a record is created, updated or deleted."
          />
        ) : (
          <div className="flex flex-col gap-3">
            {triggers.map((trigger, i) => {
              const events = eventsOf(trigger);
              const collectionProblem = problems[`trigger-${i}-collection`];
              const eventsProblem = problems[`trigger-${i}-events`];
              const knownCollection = collectionNames.includes(trigger.collection);
              return (
                <div key={i} className="flex flex-col gap-3 rounded-lg border border-border bg-surface-sunken p-3">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-medium text-muted-foreground">Trigger {i + 1}</span>
                      <ToggleSetting
                        id={`push-trigger-${i}-enabled`}
                        checked={trigger.enabled}
                        onChange={(enabled) => patchTrigger(i, { enabled })}
                        label={trigger.enabled ? "Enabled" : "Disabled"}
                      />
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Remove trigger ${i + 1}`}
                      title="Remove trigger"
                      onClick={() => patch({ triggers: triggers.filter((_, j) => j !== i) })}
                    >
                      <Trash2 className="text-destructive" />
                    </Button>
                  </div>
                  <SettingRow label="Collection" htmlFor={`push-trigger-${i}-collection`} error={collectionProblem}>
                    {collectionNames.length > 0 && (knownCollection || !trigger.collection) ? (
                      <Select value={trigger.collection} onValueChange={(collection) => patchTrigger(i, { collection })}>
                        <SelectTrigger
                          id={`push-trigger-${i}-collection`}
                          className="w-full font-mono"
                          aria-invalid={collectionProblem ? true : undefined}
                        >
                          <SelectValue placeholder="Select a collection…" />
                        </SelectTrigger>
                        <SelectContent>
                          {collectionNames.map((name) => (
                            <SelectItem key={name} value={name} className="font-mono">
                              {name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    ) : (
                      <TextSetting
                        id={`push-trigger-${i}-collection`}
                        value={trigger.collection}
                        onChange={(collection) => patchTrigger(i, { collection })}
                        invalid={Boolean(collectionProblem)}
                        mono
                      />
                    )}
                  </SettingRow>
                  <SettingRow label="Events" error={eventsProblem}>
                    <div
                      role="group"
                      aria-label={`Trigger ${i + 1} events`}
                      className="flex flex-wrap items-center gap-x-4 gap-y-1 pt-1.5"
                    >
                      {TRIGGER_EVENTS.map((event) => (
                        <label key={event} className="flex cursor-pointer items-center gap-1.5 text-sm capitalize">
                          <Checkbox
                            checked={events.includes(event)}
                            onCheckedChange={(checked) => toggleTriggerEvent(i, trigger, event, checked === true)}
                          />
                          {event}
                        </label>
                      ))}
                    </div>
                  </SettingRow>
                  <SettingRow
                    label="Title"
                    htmlFor={`push-trigger-${i}-title`}
                    help="Use {{field}} to insert a value from the record."
                  >
                    <TextSetting
                      id={`push-trigger-${i}-title`}
                      value={trigger.title}
                      onChange={(title) => patchTrigger(i, { title })}
                      placeholder="New {{title}}"
                    />
                  </SettingRow>
                  <SettingRow label="Body" htmlFor={`push-trigger-${i}-body`}>
                    <TextSetting
                      id={`push-trigger-${i}-body`}
                      value={trigger.body}
                      onChange={(body) => patchTrigger(i, { body })}
                      placeholder="{{body}}"
                    />
                  </SettingRow>
                  <SettingRow
                    label="Target field"
                    htmlFor={`push-trigger-${i}-target`}
                    help="Empty broadcasts to every subscription. Otherwise name a field on the triggering record that must equal a subscription's recordRef."
                  >
                    <TextSetting
                      id={`push-trigger-${i}-target`}
                      value={trigger.targetField}
                      onChange={(targetField) => patchTrigger(i, { targetField })}
                      placeholder="Broadcast to all"
                      mono
                    />
                  </SettingRow>
                </div>
              );
            })}
          </div>
        )}
      </SettingsSection>

      <DevicesSection unsavedProviders={dirty} />

      <SettingsSaveBar
        dirty={dirty}
        pending={save.isPending}
        errors={errors}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </SettingsPage>
  );
}

/** Multiline write-only secret, for the pasted-in-full FCM service-account
 * JSON and APNs `.p8` key. `SecretSetting` is a single-line
 * `<input type="password">`; a `<Textarea>` has no `password` mode, so this
 * keeps the same never-render-the-stored-value contract (blank means "keep
 * what is stored") on top of the shared `Textarea`. */
function SecretTextarea({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <Textarea
      id={id}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={STORED_HINT}
      autoComplete="off"
      spellCheck={false}
      className="min-h-24 font-mono text-xs"
    />
  );
}

/** A `_push_subscriptions` row. `token` is hidden by the server. */
interface PushSubscription {
  id: string;
  collectionRef: string;
  recordRef: string;
  platform: string;
  enabled: boolean;
  created: string;
}

interface PushSendResult {
  sent: number;
  failed: number;
  results: { id: string; platform: string; ok: boolean; error?: string }[];
}

const PLATFORM_LABEL: Record<string, string> = { web: "Web", fcm: "Android", android: "Android", apns: "iOS", ios: "iOS" };

/** Registered devices with a per-device test send (`POST /api/push/send`,
 * which uses the *saved* provider settings). */
function DevicesSection({ unsavedProviders }: { unsavedProviders: boolean }) {
  const [target, setTarget] = useState<PushSubscription | null>(null);
  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["push-subscriptions"],
    queryFn: () =>
      cb.collection("_push_subscriptions").list({ page: 1, perPage: 50, sort: "-created" }) as unknown as Promise<{
        items: PushSubscription[];
        totalItems: number;
      }>,
  });
  const devices = data?.items ?? [];

  return (
    <SettingsSection
      title="Devices"
      description="Subscriptions clients have registered. Send a test to confirm a provider is wired up end to end."
    >
      {error ? (
        <ErrorState title="Couldn't load devices" error={error} onRetry={() => void refetch()} compact />
      ) : isPending ? (
        <ListSkeleton rows={3} />
      ) : devices.length === 0 ? (
        <EmptyState
          compact
          icon={<Smartphone />}
          title="No devices registered"
          description="Clients register by creating a _push_subscriptions record with a platform and delivery token. They'll appear here."
        />
      ) : (
        <>
          <div className="overflow-x-auto rounded-lg border border-border bg-card">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Platform</TableHead>
                  <TableHead>Owner</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Registered</TableHead>
                  <TableHead className="w-28 text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {devices.map((device) => (
                  <TableRow key={device.id}>
                    <TableCell>
                      <Badge variant="outline" className="font-normal">
                        {PLATFORM_LABEL[device.platform] ?? device.platform}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {device.recordRef ? `${device.collectionRef}/${device.recordRef}` : "Anonymous"}
                    </TableCell>
                    <TableCell>
                      <StatusBadge tone={device.enabled ? "success" : "warning"}>
                        {device.enabled ? "Active" : "Disabled"}
                      </StatusBadge>
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {parseServerDate(device.created).toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        type="button"
                        variant="ghost"
                        size="xs"
                        aria-label={`Send test notification to ${PLATFORM_LABEL[device.platform] ?? device.platform} device ${device.id}`}
                        onClick={() => setTarget(device)}
                      >
                        <Send />
                        Test
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
          <p className="text-xs text-muted-foreground">
            Showing <span className="font-tabular">{devices.length}</span> of{" "}
            <span className="font-tabular">{data?.totalItems ?? devices.length}</span> devices.
          </p>
        </>
      )}
      {target ? (
        <TestPushDialog device={target} unsavedProviders={unsavedProviders} onClose={() => setTarget(null)} />
      ) : null}
    </SettingsSection>
  );
}

function TestPushDialog({
  device,
  unsavedProviders,
  onClose,
}: {
  device: PushSubscription;
  unsavedProviders: boolean;
  onClose: () => void;
}) {
  const [title, setTitle] = useState("Test notification");
  const [body, setBody] = useState("If you can read this, push delivery works.");
  const send = useMutation({
    mutationFn: () =>
      cb.send<PushSendResult>("/api/push/send", {
        method: "POST",
        body: { subscriptionIds: [device.id], title, body },
      }),
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't send test: ${described.title}`, { description: described.detail });
    },
  });
  const result = send.data?.results[0];

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-md">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            if ((title.trim() || body.trim()) && !send.isPending) send.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>Send a test notification</DialogTitle>
            <DialogDescription>
              To one {PLATFORM_LABEL[device.platform] ?? device.platform} device ·{" "}
              <span className="font-mono">{device.id}</span>
            </DialogDescription>
          </DialogHeader>
          {unsavedProviders ? (
            <p role="status" className="text-xs text-warning">
              Tests use the saved provider settings — save your changes first to include them.
            </p>
          ) : null}
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="push-test-title">Title</Label>
            <Input id="push-test-title" value={title} onChange={(e) => setTitle(e.target.value)} autoFocus />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="push-test-body">Body</Label>
            <Input id="push-test-body" value={body} onChange={(e) => setBody(e.target.value)} />
          </div>
          {send.data ? (
            <div role="status" className="flex flex-col gap-1">
              {send.data.results.length === 0 ? (
                <StatusBadge tone="warning">No deliverable subscription</StatusBadge>
              ) : result?.ok ? (
                <StatusBadge tone="success">Delivered to provider</StatusBadge>
              ) : (
                <StatusBadge tone="danger">Failed</StatusBadge>
              )}
              {result?.error ? <p className="text-xs break-words text-destructive">{result.error}</p> : null}
            </div>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              {send.data ? "Close" : "Cancel"}
            </Button>
            <Button type="submit" disabled={send.isPending || (!title.trim() && !body.trim())}>
              {send.isPending ? <Spinner /> : <Send />}
              {send.data ? "Send again" : "Send test"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
