import { useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { TestTube } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsApplicationRoute, type ApplicationTab } from "@/routes/settings-application";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  NumberSetting,
  SecretSetting,
  SettingRow,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

/** The slice of settings this page owns. Kept as one draft across all
 * four tabs (General/Branding/Modules/Storage) — they're small enough, and
 * share a save bar, that splitting the draft per-tab would only add
 * friction (switching tabs with unsaved edits would need to warn, or lose
 * them). `s3`/`storage` (file storage + its limits) joined the other
 * three when they moved here from Email → Delivery. */
type Draft = Pick<ServerSettings, "meta" | "batch" | "teams" | "queue" | "zipExport" | "s3" | "storage">;

function draftOf(settings: ServerSettings): Draft {
  return {
    meta: { ...settings.meta },
    batch: { ...settings.batch },
    teams: { ...settings.teams },
    queue: { ...settings.queue },
    zipExport: { ...settings.zipExport },
    s3: { ...settings.s3, secret: "" },
    storage: { ...settings.storage },
  };
}

/** Only send the S3 secret when one was typed — an empty box means "keep
 * what is stored", not "clear it" (same convention as the SMTP password
 * on the Email → Delivery tab). */
function payloadOf(draft: Draft) {
  const s3: Record<string, unknown> = { ...draft.s3 };
  if (!draft.s3.secret) delete s3.secret;
  return {
    meta: draft.meta,
    batch: draft.batch,
    teams: draft.teams,
    queue: draft.queue,
    zipExport: draft.zipExport,
    s3,
    storage: draft.storage,
  };
}

/** The same rules the server applies, so a bad value is named here rather
 * than coming back as a 400 with a nested `data` envelope. */
function validate(draft: Draft): string[] {
  const errors: string[] = [];
  if (!draft.meta.appName.trim()) errors.push("Application name is required");
  if (draft.meta.appURL.trim()) {
    try {
      new URL(draft.meta.appURL);
    } catch {
      errors.push("Application URL must be a full URL, e.g. https://example.com");
    }
  }
  if (draft.meta.senderAddress.trim() && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(draft.meta.senderAddress)) {
    errors.push("Sender address must be an email address");
  }
  if (draft.meta.logoUrl.trim()) {
    try {
      new URL(draft.meta.logoUrl);
    } catch {
      errors.push("Logo URL must be a full URL, e.g. https://example.com/logo.png");
    }
  }
  const hexColor = /^#[0-9a-fA-F]{3,8}$/;
  if (draft.meta.brandColor.trim() && !hexColor.test(draft.meta.brandColor.trim())) {
    errors.push("Brand color must be a hex color, e.g. #1055c9");
  }
  if (draft.meta.accentColor.trim() && !hexColor.test(draft.meta.accentColor.trim())) {
    errors.push("Accent color must be a hex color, e.g. #1055c9");
  }
  if (draft.batch.maxRequests < 1) errors.push("A batch must allow at least one request");
  if (draft.batch.timeout < 1) errors.push("Batch timeout must be at least 1 second");
  if (draft.batch.maxBodySize < 0) errors.push("Batch max body size can't be negative");
  if (draft.s3.enabled) {
    if (!draft.s3.bucket.trim()) errors.push("S3 needs a bucket");
    if (!draft.s3.endpoint.trim()) errors.push("S3 needs an endpoint");
  }
  if (draft.storage.maxTransformDimension < 0) errors.push("Max transform dimension can't be negative");
  if (draft.storage.userQuotaBytes < 0) errors.push("Storage quota can't be negative");
  return errors;
}

function dirtyAgainst(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

export function ApplicationPage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
  const testS3 = useMutation({
    mutationFn: (which: "storage" | "backups") => cb.admin.settings.testS3(which),
    onSuccess: () => toast.success("S3 reachable", { description: "The bucket answered." }),
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error("S3 test failed", { description: failure.serverMessage || failure.detail });
    },
  });
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(settings);
  const search = settingsApplicationRoute.useSearch();
  const navigate = settingsApplicationRoute.useNavigate();
  const tab: ApplicationTab = search.tab ?? "general";

  // Adopt server state on first load and on any refetch that lands while
  // nothing is being edited; never clobber an edit in progress.
  if (settings && (seedKey !== settings || draft === null)) {
    if (draft === null || seedKey === undefined || !dirtyAgainst(draft, seedKey)) {
      setSeedKey(settings);
      setDraft(draftOf(settings));
    } else if (seedKey !== settings) {
      setSeedKey(settings);
    }
  }

  const errors = useMemo(() => (draft ? validate(draft) : []), [draft]);

  if (isPending || !draft || !settings) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={i} className="h-40 w-full" />
        ))}
      </div>
    );
  }

  const dirty = dirtyAgainst(draft, settings);

  function patch(next: Partial<Draft>) {
    setDraft((d) => (d ? { ...d, ...next } : d));
  }

  function submit() {
    if (!draft || errors.length > 0) return;
    save.mutate(payloadOf(draft), {
      onSuccess: () => {
        toast.success("Settings saved");
        // The S3 secret was consumed; clear the box so it reads as "stored".
        setDraft((d) => (d ? { ...d, s3: { ...d.s3, secret: "" } } : d));
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
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page pb-24">
      <div className="flex flex-col">
        <h1 className="text-base font-medium tracking-tight">Application</h1>
        <p className="mt-0.5 max-w-measure text-sm text-muted-foreground">
          How this instance identifies itself, its branding, which optional modules are on, and file storage.
        </p>
      </div>

      <Tabs value={tab} onValueChange={(next) => void navigate({ search: { tab: next as ApplicationTab } })}>
        <TabsList>
          <TabsTrigger value="general">General</TabsTrigger>
          <TabsTrigger value="branding">Branding</TabsTrigger>
          <TabsTrigger value="modules">Modules</TabsTrigger>
          <TabsTrigger value="storage">Storage</TabsTrigger>
        </TabsList>

        <TabsContent value="general" className="pt-2">
          <SettingsSection title="Application" description="How this instance identifies itself in emails and links.">
            <SettingRow label="Name" htmlFor="app-name">
              <TextSetting
                id="app-name"
                value={draft.meta.appName}
                onChange={(appName) => patch({ meta: { ...draft.meta, appName } })}
                placeholder="Acme"
              />
            </SettingRow>
            <SettingRow
              label="URL"
              htmlFor="app-url"
              help="The public address of this server. Used to build links in system emails."
            >
              <TextSetting
                id="app-url"
                value={draft.meta.appURL}
                onChange={(appURL) => patch({ meta: { ...draft.meta, appURL } })}
                placeholder="https://example.com"
              />
            </SettingRow>
            <SettingRow label="Sender name" htmlFor="sender-name">
              <TextSetting
                id="sender-name"
                value={draft.meta.senderName}
                onChange={(senderName) => patch({ meta: { ...draft.meta, senderName } })}
                placeholder="Support"
              />
            </SettingRow>
            <SettingRow label="Sender address" htmlFor="sender-address" help="The From address on system emails.">
              <TextSetting
                id="sender-address"
                type="email"
                value={draft.meta.senderAddress}
                onChange={(senderAddress) => patch({ meta: { ...draft.meta, senderAddress } })}
                placeholder="support@example.com"
              />
            </SettingRow>
            <SettingRow
              label="Hide controls"
              htmlFor="app-hide-controls"
              help="Hides sensitive configuration controls (SMTP, OAuth2, storage, ...) from other superusers in this dashboard — useful once those are managed outside the UI (env vars, infrastructure-as-code) and shouldn't be edited here by accident."
            >
              <ToggleSetting
                id="app-hide-controls"
                checked={draft.meta.hideControls}
                onChange={(hideControls) => patch({ meta: { ...draft.meta, hideControls } })}
                label={draft.meta.hideControls ? "Hidden from other superusers" : "Visible to every superuser"}
              />
            </SettingRow>
          </SettingsSection>
        </TabsContent>

        <TabsContent value="branding" className="pt-2">
          <SettingsSection
            title="Branding"
            description="Used in system emails and as the email-template editor's default theme."
          >
            <SettingRow
              label="Logo URL"
              htmlFor="app-logo-url"
              help="Shown at the top of system emails and as the email-template editor's default theme logo. Left blank, a text wordmark of the application name is used instead."
            >
              <TextSetting
                id="app-logo-url"
                value={draft.meta.logoUrl}
                onChange={(logoUrl) => patch({ meta: { ...draft.meta, logoUrl } })}
                placeholder="https://example.com/logo.png"
              />
            </SettingRow>
            <SettingRow
              label="Brand color"
              htmlFor="app-brand-color"
              help="The call-to-action color in system emails and the email-template editor's default theme. Left blank, a neutral default is used."
            >
              <TextSetting
                id="app-brand-color"
                value={draft.meta.brandColor}
                onChange={(brandColor) => patch({ meta: { ...draft.meta, brandColor } })}
                placeholder="#1055c9"
                mono
              />
            </SettingRow>
            <SettingRow
              label="Accent color"
              htmlFor="app-accent-color"
              help="The dashboard's own accent color for this instance. Left blank, the default accent is used."
            >
              <TextSetting
                id="app-accent-color"
                value={draft.meta.accentColor}
                onChange={(accentColor) => patch({ meta: { ...draft.meta, accentColor } })}
                placeholder="#1055c9"
                mono
              />
            </SettingRow>
          </SettingsSection>
        </TabsContent>

        <TabsContent value="modules" className="pt-2">
          <SettingsSection
            title="Batch API"
            description="POST /api/batch runs many writes in one transaction. The dashboard's bulk delete needs it."
          >
            <SettingRow label="Enabled" htmlFor="batch-enabled">
              <ToggleSetting
                id="batch-enabled"
                checked={draft.batch.enabled}
                onChange={(enabled) => patch({ batch: { ...draft.batch, enabled } })}
                label={draft.batch.enabled ? "Accepting batch requests" : "Rejecting batch requests with 403"}
              />
            </SettingRow>
            <SettingRow
              label="Max requests"
              htmlFor="batch-max"
              help="How many operations one batch may carry. The grid chunks a larger selection to fit."
            >
              <NumberSetting
                id="batch-max"
                min={1}
                value={draft.batch.maxRequests}
                onChange={(maxRequests) => patch({ batch: { ...draft.batch, maxRequests } })}
                suffix="operations"
              />
            </SettingRow>
            <SettingRow label="Timeout" htmlFor="batch-timeout">
              <NumberSetting
                id="batch-timeout"
                min={1}
                value={draft.batch.timeout}
                onChange={(timeout) => patch({ batch: { ...draft.batch, timeout } })}
                suffix="seconds"
              />
            </SettingRow>
            <SettingRow
              label="Max body size"
              htmlFor="batch-max-body"
              help="Reject a batch request whose raw body is larger than this, in bytes. 0 means no limit beyond the server's own request-size cap."
            >
              <NumberSetting
                id="batch-max-body"
                min={0}
                value={draft.batch.maxBodySize}
                onChange={(maxBodySize) => patch({ batch: { ...draft.batch, maxBodySize } })}
                suffix="bytes"
              />
            </SettingRow>
          </SettingsSection>

          <SettingsSection
            title="Modules"
            description="Built into the server binary either way, but off by default and hidden from the dashboard until enabled."
          >
            <SettingRow
              label="Teams"
              htmlFor="module-teams"
              help="Shared ownership over records via _teams/_team_members. Takes effect immediately — no restart needed."
            >
              <ToggleSetting
                id="module-teams"
                checked={draft.teams.enabled}
                onChange={(enabled) => patch({ teams: { enabled } })}
                label={draft.teams.enabled ? "Enabled" : "Disabled"}
              />
            </SettingRow>
            <SettingRow
              label="Queue"
              htmlFor="module-queue"
              help="Background job queue (_queue_jobs). Requires a server restart after enabling or disabling — the worker and its route/hook wiring are only assembled at boot."
            >
              <ToggleSetting
                id="module-queue"
                checked={draft.queue.enabled}
                onChange={(enabled) => patch({ queue: { enabled } })}
                label={draft.queue.enabled ? "Enabled — restart to apply" : "Disabled"}
              />
            </SettingRow>
            <SettingRow
              label="ZIP export"
              htmlFor="module-zip-export"
              help="Download a filtered set of files from a collection as one ZIP (_zip_exports). Requires a server restart after enabling or disabling, same as Queue."
            >
              <ToggleSetting
                id="module-zip-export"
                checked={draft.zipExport.enabled}
                onChange={(enabled) => patch({ zipExport: { enabled } })}
                label={draft.zipExport.enabled ? "Enabled — restart to apply" : "Disabled"}
              />
            </SettingRow>
          </SettingsSection>
        </TabsContent>

        <TabsContent value="storage" className="pt-2 flex flex-col gap-4">
          <SettingsSection
            title="File storage"
            description="Where uploaded files live. Off keeps them on the server's own disk under the data directory."
            action={
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-control-sm shrink-0 gap-1.5"
                disabled={testS3.isPending}
                onClick={() => testS3.mutate("storage")}
              >
                {testS3.isPending ? <Spinner /> : <TestTube className="size-3.5" />}
                Test connection
              </Button>
            }
          >
            <SettingRow label="Use S3" htmlFor="s3-enabled">
              <ToggleSetting
                id="s3-enabled"
                checked={draft.s3.enabled}
                onChange={(enabled) => patch({ s3: { ...draft.s3, enabled } })}
                label={draft.s3.enabled ? "Uploads go to S3" : "Uploads stay on local disk"}
              />
            </SettingRow>
            <SettingRow label="Bucket" htmlFor="s3-bucket">
              <TextSetting
                id="s3-bucket"
                value={draft.s3.bucket}
                onChange={(bucket) => patch({ s3: { ...draft.s3, bucket } })}
                mono
              />
            </SettingRow>
            <SettingRow label="Region" htmlFor="s3-region">
              <TextSetting
                id="s3-region"
                value={draft.s3.region}
                onChange={(region) => patch({ s3: { ...draft.s3, region } })}
                placeholder="us-east-1"
                mono
              />
            </SettingRow>
            <SettingRow label="Endpoint" htmlFor="s3-endpoint" help="Any S3-compatible endpoint — R2, MinIO, Spaces.">
              <TextSetting
                id="s3-endpoint"
                value={draft.s3.endpoint}
                onChange={(endpoint) => patch({ s3: { ...draft.s3, endpoint } })}
                placeholder="https://s3.amazonaws.com"
                mono
              />
            </SettingRow>
            <SettingRow label="Access key" htmlFor="s3-key">
              <TextSetting
                id="s3-key"
                value={draft.s3.accessKey}
                onChange={(accessKey) => patch({ s3: { ...draft.s3, accessKey } })}
                mono
              />
            </SettingRow>
            <SettingRow label="Secret" htmlFor="s3-secret" help="Never sent back. Leave blank to keep the stored one.">
              <SecretSetting
                id="s3-secret"
                value={draft.s3.secret ?? ""}
                onChange={(secret) => patch({ s3: { ...draft.s3, secret } })}
                storedHint="•••••••• (unchanged)"
              />
            </SettingRow>
            <SettingRow
              label="Path-style URLs"
              htmlFor="s3-path"
              help="Needed by MinIO and some self-hosted gateways that don't support virtual-hosted buckets."
            >
              <ToggleSetting
                id="s3-path"
                checked={draft.s3.forcePathStyle}
                onChange={(forcePathStyle) => patch({ s3: { ...draft.s3, forcePathStyle } })}
                label={draft.s3.forcePathStyle ? "bucket in the path" : "bucket in the hostname"}
              />
            </SettingRow>
          </SettingsSection>

          <SettingsSection
            title="Storage limits"
            description="Image transforms and the per-user storage quota — both apply whether files live on local disk or S3."
          >
            <SettingRow
              label="Image transforms"
              htmlFor="storage-transforms-enabled"
              help="Whether ?w=/?h=/?fit=/?format=/?q= are honored on the files route. ?thumb= is unaffected either way."
            >
              <ToggleSetting
                id="storage-transforms-enabled"
                checked={draft.storage.imageTransformsEnabled}
                onChange={(imageTransformsEnabled) =>
                  patch({ storage: { ...draft.storage, imageTransformsEnabled } })
                }
                label={draft.storage.imageTransformsEnabled ? "Transforms enabled" : "Transforms disabled"}
              />
            </SettingRow>
            <SettingRow
              label="Max transform dimension"
              htmlFor="storage-max-dimension"
              help="The largest ?w=/?h= a non-superuser request may ask for, in pixels. 0 means no limit."
            >
              <NumberSetting
                id="storage-max-dimension"
                min={0}
                suffix="px (0 = no limit)"
                value={draft.storage.maxTransformDimension}
                onChange={(maxTransformDimension) =>
                  patch({ storage: { ...draft.storage, maxTransformDimension } })
                }
              />
            </SettingRow>
            <SettingRow
              label="Per-user storage quota"
              htmlFor="storage-quota"
              help="Caps the total bytes a single auth record may store across every collection with an ownerField set (see that collection's settings). 0 disables the quota."
            >
              <NumberSetting
                id="storage-quota"
                min={0}
                suffix="bytes (0 = unlimited)"
                value={draft.storage.userQuotaBytes}
                onChange={(userQuotaBytes) => patch({ storage: { ...draft.storage, userQuotaBytes } })}
              />
            </SettingRow>
          </SettingsSection>
        </TabsContent>
      </Tabs>

      <SettingsSaveBar
        dirty={dirty}
        pending={save.isPending}
        errors={errors}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </div>
  );
}
