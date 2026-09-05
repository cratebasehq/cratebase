import { useMemo, useState } from "react";
import { toast } from "sonner";
import { Plus, Trash2 } from "lucide-react";
import { describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import {
  SettingRow,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

type Push = ServerSettings["push"];
type PushTrigger = Push["triggers"][number];

type Draft = Pick<ServerSettings, "push">;

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

function validate(draft: Draft): string[] {
  const errors: string[] = [];
  if (draft.push.vapid.enabled) {
    if (!draft.push.vapid.publicKey.trim()) errors.push("VAPID needs a public key");
    if (!draft.push.vapid.subject.trim()) errors.push("VAPID needs a subject");
  }
  if (draft.push.apns.enabled) {
    if (!draft.push.apns.keyId.trim()) errors.push("APNs needs a key ID");
    if (!draft.push.apns.teamId.trim()) errors.push("APNs needs a team ID");
    if (!draft.push.apns.bundleId.trim()) errors.push("APNs needs a bundle ID");
  }
  draft.push.triggers.forEach((trigger, i) => {
    if (!trigger.enabled) return;
    if (!trigger.collection.trim()) errors.push(`Trigger ${i + 1} needs a collection`);
    if (!trigger.events.trim()) errors.push(`Trigger ${i + 1} needs at least one event`);
  });
  return errors;
}

function emptyTrigger(): PushTrigger {
  return { enabled: true, collection: "", events: "", title: "", body: "", targetField: "" };
}

export function PushPage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(undefined);

  if (settings && (draft === null || (seedKey !== settings && !isDirty(draft, settings)))) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  const errors = useMemo(() => (draft ? validate(draft) : []), [draft]);

  if (isPending || !draft || !settings) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-48 w-full" />
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

  function addTrigger() {
    if (!draft) return;
    patch({ triggers: [...draft.push.triggers, emptyTrigger()] });
  }

  function removeTrigger(index: number) {
    if (!draft) return;
    patch({ triggers: draft.push.triggers.filter((_, i) => i !== index) });
  }

  function submit() {
    if (!draft || errors.length > 0) return;
    save.mutate(payloadOf(draft), {
      onSuccess: () => {
        toast.success("Settings saved");
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
        toast.error(failure.title, {
          description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
        });
      },
    });
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
      <div className="flex flex-col gap-1">
        <h2 className="text-lg font-medium">Push notifications</h2>
        <p className="max-w-measure text-sm text-muted-foreground">
          Configures push notification delivery for the <code className="font-mono">_push_subscriptions</code>{" "}
          collection across three channels. A subscription's own <code className="font-mono">platform</code>{" "}
          field (<code className="font-mono">web</code>/<code className="font-mono">android</code>/
          <code className="font-mono">ios</code>) picks which of the three channels below serves it.
        </p>
      </div>

      <SettingsSection
        title="Web Push (VAPID)"
        description="Delivers to browsers subscribed with the Push API. No per-platform app registration needed."
      >
        <SettingRow label="Enabled" htmlFor="push-vapid-enabled">
          <ToggleSetting
            id="push-vapid-enabled"
            checked={draft.push.vapid.enabled}
            onChange={(enabled) => patch({ vapid: { ...draft.push.vapid, enabled } })}
            label={draft.push.vapid.enabled ? "Sending web push" : "Not sending web push"}
          />
        </SettingRow>
        <SettingRow label="Public key" htmlFor="push-vapid-public">
          <TextSetting
            id="push-vapid-public"
            value={draft.push.vapid.publicKey}
            onChange={(publicKey) => patch({ vapid: { ...draft.push.vapid, publicKey } })}
            mono
          />
        </SettingRow>
        <SettingRow
          label="Private key"
          htmlFor="push-vapid-private"
          help="Never sent back. Leave blank to keep the stored one."
        >
          <LocalSecretSetting
            id="push-vapid-private"
            value={draft.push.vapid.privateKey ?? ""}
            onChange={(privateKey) => patch({ vapid: { ...draft.push.vapid, privateKey } })}
            storedHint="•••••••• (unchanged)"
          />
        </SettingRow>
        <SettingRow
          label="Subject"
          htmlFor="push-vapid-subject"
          help="A contact URI push services can use to reach the operator, e.g. mailto:ops@example.com."
        >
          <TextSetting
            id="push-vapid-subject"
            value={draft.push.vapid.subject}
            onChange={(subject) => patch({ vapid: { ...draft.push.vapid, subject } })}
            placeholder="mailto:ops@example.com"
            mono
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Firebase Cloud Messaging (Android)"
        description="Delivers to Android devices via the FCM HTTP v1 API."
      >
        <SettingRow label="Enabled" htmlFor="push-fcm-enabled">
          <ToggleSetting
            id="push-fcm-enabled"
            checked={draft.push.fcm.enabled}
            onChange={(enabled) => patch({ fcm: { ...draft.push.fcm, enabled } })}
            label={draft.push.fcm.enabled ? "Sending FCM push" : "Not sending FCM push"}
          />
        </SettingRow>
        <SettingRow
          label="Service account JSON"
          htmlFor="push-fcm-sa"
          help="Never sent back. Leave blank to keep the stored one. The full contents of the Firebase service-account key file."
        >
          <LocalSecretTextarea
            id="push-fcm-sa"
            value={draft.push.fcm.serviceAccountJson ?? ""}
            onChange={(serviceAccountJson) => patch({ fcm: { ...draft.push.fcm, serviceAccountJson } })}
            storedHint="•••••••• (unchanged)"
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Apple Push Notifications (iOS)"
        description="Delivers to iOS devices via the APNs HTTP/2 provider API, using token-based (.p8) auth."
      >
        <SettingRow label="Enabled" htmlFor="push-apns-enabled">
          <ToggleSetting
            id="push-apns-enabled"
            checked={draft.push.apns.enabled}
            onChange={(enabled) => patch({ apns: { ...draft.push.apns, enabled } })}
            label={draft.push.apns.enabled ? "Sending APNs push" : "Not sending APNs push"}
          />
        </SettingRow>
        <SettingRow
          label="Key (.p8)"
          htmlFor="push-apns-key"
          help="Never sent back. Leave blank to keep the stored one."
        >
          <LocalSecretTextarea
            id="push-apns-key"
            value={draft.push.apns.key ?? ""}
            onChange={(key) => patch({ apns: { ...draft.push.apns, key } })}
            storedHint="•••••••• (unchanged)"
          />
        </SettingRow>
        <SettingRow label="Key ID" htmlFor="push-apns-keyid">
          <TextSetting
            id="push-apns-keyid"
            value={draft.push.apns.keyId}
            onChange={(keyId) => patch({ apns: { ...draft.push.apns, keyId } })}
            mono
          />
        </SettingRow>
        <SettingRow label="Team ID" htmlFor="push-apns-teamid">
          <TextSetting
            id="push-apns-teamid"
            value={draft.push.apns.teamId}
            onChange={(teamId) => patch({ apns: { ...draft.push.apns, teamId } })}
            mono
          />
        </SettingRow>
        <SettingRow label="Bundle ID" htmlFor="push-apns-bundleid">
          <TextSetting
            id="push-apns-bundleid"
            value={draft.push.apns.bundleId}
            onChange={(bundleId) => patch({ apns: { ...draft.push.apns, bundleId } })}
            placeholder="com.example.app"
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
            checked={draft.push.apns.production}
            onChange={(production) => patch({ apns: { ...draft.push.apns, production } })}
            label={draft.push.apns.production ? "api.push.apple.com" : "api.sandbox.push.apple.com"}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Triggers"
        description="Record-event → push rules. Each enabled trigger fires a push when a matching event happens on the collection below."
        action={
          <Button type="button" variant="outline" size="sm" className="h-control-sm gap-1.5" onClick={addTrigger}>
            <Plus className="size-3.5" />
            Add trigger
          </Button>
        }
      >
        {draft.push.triggers.length === 0 ? (
          <p className="text-xs text-muted-foreground">No triggers — records never push automatically.</p>
        ) : (
          <div className="flex flex-col gap-3">
            {draft.push.triggers.map((trigger, i) => (
              <div key={i} className="flex flex-col gap-2 rounded-lg border border-border p-2.5">
                <div className="flex items-center justify-between gap-2">
                  <ToggleSetting
                    id={`push-trigger-${i}-enabled`}
                    checked={trigger.enabled}
                    onChange={(enabled) => patchTrigger(i, { enabled })}
                    label={trigger.enabled ? "Enabled" : "Disabled"}
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Remove trigger ${i + 1}`}
                    onClick={() => removeTrigger(i)}
                  >
                    <Trash2 className="size-3.5 text-destructive" />
                  </Button>
                </div>
                <SettingRow label="Collection" htmlFor={`push-trigger-${i}-collection`}>
                  <TextSetting
                    id={`push-trigger-${i}-collection`}
                    value={trigger.collection}
                    onChange={(collection) => patchTrigger(i, { collection })}
                    mono
                  />
                </SettingRow>
                <SettingRow label="Events" htmlFor={`push-trigger-${i}-events`}>
                  <TextSetting
                    id={`push-trigger-${i}-events`}
                    value={trigger.events}
                    onChange={(events) => patchTrigger(i, { events })}
                    placeholder="create,update"
                    mono
                  />
                </SettingRow>
                <SettingRow label="Title" htmlFor={`push-trigger-${i}-title`}>
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
                  help="Empty broadcasts to every subscription. Non-empty names a field on the triggering record that must equal a subscription's recordRef."
                >
                  <TextSetting
                    id={`push-trigger-${i}-target`}
                    value={trigger.targetField}
                    onChange={(targetField) => patchTrigger(i, { targetField })}
                    mono
                  />
                </SettingRow>
              </div>
            ))}
          </div>
        )}
      </SettingsSection>

      <SettingsSaveBar
        dirty={isDirty(draft, settings)}
        pending={save.isPending}
        errors={errors}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </div>
  );
}

function isDirty(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

/** Single-line write-only secret. Identical to `SecretSetting` in
 * `settings-form.tsx`, kept local because this page's plain-text APNs/VAPID
 * keys don't need masking beyond what an `<input type="password">` already
 * gives — reused here only so the id/value/onChange/storedHint contract
 * stays consistent with the multiline variant below. */
function LocalSecretSetting({
  id,
  value,
  onChange,
  storedHint,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  storedHint: string;
}) {
  return (
    <input
      id={id}
      type="password"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={storedHint}
      autoComplete="new-password"
      className="flex h-control-md w-full rounded-lg border border-input bg-transparent px-2.5 text-sm outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 dark:bg-input/30"
    />
  );
}

/** Multiline write-only secret, for the pasted-in-full FCM service-account
 * JSON and APNs `.p8` key. `SecretSetting` in `settings-form.tsx` is a
 * single-line `<input type="password">`; a `<Textarea>` has no `password`
 * mode, so this hand-rolls the same never-render-the-stored-value contract
 * (blank means "keep what is stored") on top of the shared `Textarea`
 * component instead of stretching `SecretSetting` to cover a shape it
 * wasn't built for. */
function LocalSecretTextarea({
  id,
  value,
  onChange,
  storedHint,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  storedHint: string;
}) {
  return (
    <Textarea
      id={id}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={storedHint}
      autoComplete="off"
      spellCheck={false}
      className="min-h-24 font-mono text-xs"
    />
  );
}
