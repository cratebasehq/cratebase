import { useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { TestTube } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsApplicationRoute, type ApplicationTab } from "@/routes/settings-application";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { StatusBadge } from "@/components/layout/stat-card";
import {
  NumberSetting,
  SecretSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";
import { useSaveShortcut } from "@/components/settings/use-save-shortcut";

const GROUP = SETTINGS_GROUPS.find((g) => g.to === "/settings/application")!;

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

function isUrl(value: string): boolean {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
}

const HEX_COLOR = /^#[0-9a-fA-F]{3,8}$/;

/** Which tab each field lives on, so a tab can flag that it holds an error. */
const FIELD_TAB: Record<string, ApplicationTab> = {
  appName: "general",
  appURL: "general",
  senderAddress: "general",
  logoUrl: "branding",
  brandColor: "branding",
  accentColor: "branding",
  batchMax: "modules",
  batchTimeout: "modules",
  batchBody: "modules",
  s3Bucket: "storage",
  s3Endpoint: "storage",
  maxDimension: "storage",
  quota: "storage",
};

/** The same rules the server applies, keyed by field so each one is named
 * next to its input rather than coming back as a 400 with a nested `data`
 * envelope. */
function fieldErrors(draft: Draft): Record<string, string> {
  const errors: Record<string, string> = {};
  if (!draft.meta.appName.trim()) errors.appName = "Application name is required";
  if (draft.meta.appURL.trim() && !isUrl(draft.meta.appURL)) {
    errors.appURL = "Must be a full URL, e.g. https://example.com";
  }
  if (draft.meta.senderAddress.trim() && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(draft.meta.senderAddress)) {
    errors.senderAddress = "Must be an email address";
  }
  if (draft.meta.logoUrl.trim() && !isUrl(draft.meta.logoUrl)) {
    errors.logoUrl = "Must be a full URL, e.g. https://example.com/logo.png";
  }
  if (draft.meta.brandColor.trim() && !HEX_COLOR.test(draft.meta.brandColor.trim())) {
    errors.brandColor = "Must be a hex color, e.g. #1055c9";
  }
  if (draft.meta.accentColor.trim() && !HEX_COLOR.test(draft.meta.accentColor.trim())) {
    errors.accentColor = "Must be a hex color, e.g. #1055c9";
  }
  if (draft.batch.maxRequests < 1) errors.batchMax = "A batch must allow at least one request";
  if (draft.batch.timeout < 1) errors.batchTimeout = "Must be at least 1 second";
  if (draft.batch.maxBodySize < 0) errors.batchBody = "Can't be negative";
  if (draft.s3.enabled) {
    if (!draft.s3.bucket.trim()) errors.s3Bucket = "S3 needs a bucket";
    if (!draft.s3.endpoint.trim()) errors.s3Endpoint = "S3 needs an endpoint";
  }
  if (draft.storage.maxTransformDimension < 0) errors.maxDimension = "Can't be negative";
  if (draft.storage.userQuotaBytes < 0) errors.quota = "Can't be negative";
  return errors;
}

function dirtyAgainst(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

/** Small preview of a colour field, shown only when the value is valid. */
function ColorSwatch({ value }: { value: string }) {
  const color = value.trim();
  if (!HEX_COLOR.test(color)) return null;
  return (
    <span
      aria-hidden
      className="size-control-md shrink-0 rounded-md border border-border shadow-xs"
      style={{ backgroundColor: color }}
    />
  );
}

/** Tab trigger label that flags a tab holding an invalid field. */
function TabLabel({ label, invalid }: { label: string; invalid: boolean }) {
  return (
    <>
      {label}
      {invalid ? (
        <>
          <span aria-hidden className="size-1.5 rounded-full bg-destructive" />
          <span className="sr-only">(has errors)</span>
        </>
      ) : null}
    </>
  );
}

function ApplicationSkeleton() {
  return (
    <SettingsPage title={GROUP.label} description={GROUP.description} width="form">
      <Skeleton className="h-8 w-72" />
      <Skeleton className="h-64 w-full" />
    </SettingsPage>
  );
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

  const fieldErrs = useMemo(() => (draft ? fieldErrors(draft) : {}), [draft]);
  const errors = Object.values(fieldErrs);
  const tabHasError = (t: ApplicationTab) => Object.keys(fieldErrs).some((k) => FIELD_TAB[k] === t);
  const dirty = draft && settings ? dirtyAgainst(draft, settings) : false;

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

  useSaveShortcut({ dirty, canSave: errors.length === 0 && !save.isPending, onSave: submit });

  if (isPending || !draft || !settings) return <ApplicationSkeleton />;

  function patch(next: Partial<Draft>) {
    setDraft((d) => (d ? { ...d, ...next } : d));
  }

  // The connection test runs against what is saved, not the draft — so it is
  // only meaningful once the S3 edits are saved.
  const s3Edited = JSON.stringify(draft.s3) !== JSON.stringify({ ...settings.s3, secret: "" });

  return (
    <SettingsPage title={GROUP.label} description={GROUP.description} width="form">
      <Tabs value={tab} onValueChange={(next) => void navigate({ search: { tab: next as ApplicationTab } })}>
        <TabsList>
          <TabsTrigger value="general">
            <TabLabel label="General" invalid={tabHasError("general")} />
          </TabsTrigger>
          <TabsTrigger value="branding">
            <TabLabel label="Branding" invalid={tabHasError("branding")} />
          </TabsTrigger>
          <TabsTrigger value="modules">
            <TabLabel label="Modules" invalid={tabHasError("modules")} />
          </TabsTrigger>
          <TabsTrigger value="storage">
            <TabLabel label="Storage" invalid={tabHasError("storage")} />
          </TabsTrigger>
        </TabsList>

        <TabsContent value="general" className="flex flex-col gap-4 pt-2">
          <SettingsSection title="Identity" description="How this instance identifies itself in emails and links.">
            <SettingRow
              label="Name"
              htmlFor="app-name"
              help="Shown in the dashboard title and in the subject and footer of system emails."
              error={fieldErrs.appName}
            >
              <TextSetting
                id="app-name"
                value={draft.meta.appName}
                onChange={(appName) => patch({ meta: { ...draft.meta, appName } })}
                placeholder="Acme"
                invalid={!!fieldErrs.appName}
              />
            </SettingRow>
            <SettingRow
              label="URL"
              htmlFor="app-url"
              help="The public address of this server. Used to build links in system emails."
              error={fieldErrs.appURL}
            >
              <TextSetting
                id="app-url"
                value={draft.meta.appURL}
                onChange={(appURL) => patch({ meta: { ...draft.meta, appURL } })}
                placeholder="https://example.com"
                mono
                invalid={!!fieldErrs.appURL}
              />
            </SettingRow>
          </SettingsSection>

          <SettingsSection title="Sender" description="The From line on every system email — verification, password reset, invitations.">
            <SettingRow label="Sender name" htmlFor="sender-name">
              <TextSetting
                id="sender-name"
                value={draft.meta.senderName}
                onChange={(senderName) => patch({ meta: { ...draft.meta, senderName } })}
                placeholder="Support"
              />
            </SettingRow>
            <SettingRow
              label="Sender address"
              htmlFor="sender-address"
              help="Must be allowed by your mail provider, or delivery will fail."
              error={fieldErrs.senderAddress}
            >
              <TextSetting
                id="sender-address"
                type="email"
                value={draft.meta.senderAddress}
                onChange={(senderAddress) => patch({ meta: { ...draft.meta, senderAddress } })}
                placeholder="support@example.com"
                invalid={!!fieldErrs.senderAddress}
              />
            </SettingRow>
          </SettingsSection>

          <SettingsSection title="Dashboard access" description="Who may see sensitive configuration in this dashboard.">
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

        <TabsContent value="branding" className="flex flex-col gap-4 pt-2">
          <SettingsSection
            title="Branding"
            description="Used in system emails and as the email-template editor's default theme."
          >
            <SettingRow
              label="Logo URL"
              htmlFor="app-logo-url"
              help="Shown at the top of system emails and as the email-template editor's default theme logo. Left blank, a text wordmark of the application name is used instead."
              error={fieldErrs.logoUrl}
            >
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <TextSetting
                    id="app-logo-url"
                    value={draft.meta.logoUrl}
                    onChange={(logoUrl) => patch({ meta: { ...draft.meta, logoUrl } })}
                    placeholder="https://example.com/logo.png"
                    mono
                    invalid={!!fieldErrs.logoUrl}
                  />
                </div>
                {draft.meta.logoUrl.trim() && !fieldErrs.logoUrl ? (
                  <img
                    src={draft.meta.logoUrl.trim()}
                    alt="Logo preview"
                    className="size-control-md shrink-0 rounded-md border border-border bg-surface-sunken object-contain p-0.5"
                    onError={(e) => (e.currentTarget.style.visibility = "hidden")}
                    onLoad={(e) => (e.currentTarget.style.visibility = "visible")}
                  />
                ) : null}
              </div>
            </SettingRow>
            <SettingRow
              label="Brand color"
              htmlFor="app-brand-color"
              help="The call-to-action color in system emails and the email-template editor's default theme. Left blank, a neutral default is used."
              error={fieldErrs.brandColor}
            >
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <TextSetting
                    id="app-brand-color"
                    value={draft.meta.brandColor}
                    onChange={(brandColor) => patch({ meta: { ...draft.meta, brandColor } })}
                    placeholder="#1055c9"
                    mono
                    invalid={!!fieldErrs.brandColor}
                  />
                </div>
                <ColorSwatch value={draft.meta.brandColor} />
              </div>
            </SettingRow>
            <SettingRow
              label="Accent color"
              htmlFor="app-accent-color"
              help="The dashboard's own accent color for this instance. Left blank, the default accent is used."
              error={fieldErrs.accentColor}
            >
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <TextSetting
                    id="app-accent-color"
                    value={draft.meta.accentColor}
                    onChange={(accentColor) => patch({ meta: { ...draft.meta, accentColor } })}
                    placeholder="#1055c9"
                    mono
                    invalid={!!fieldErrs.accentColor}
                  />
                </div>
                <ColorSwatch value={draft.meta.accentColor} />
              </div>
            </SettingRow>
          </SettingsSection>
        </TabsContent>

        <TabsContent value="modules" className="flex flex-col gap-4 pt-2">
          <SettingsSection
            title="Optional modules"
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
              help="Background job queue (_queue_jobs). The worker and its route/hook wiring are only assembled at boot, so a change needs a server restart."
            >
              <div className="flex items-center gap-2">
                <ToggleSetting
                  id="module-queue"
                  checked={draft.queue.enabled}
                  onChange={(enabled) => patch({ queue: { enabled } })}
                  label={draft.queue.enabled ? "Enabled" : "Disabled"}
                />
                {draft.queue.enabled !== settings.queue.enabled ? (
                  <StatusBadge tone="warning">Restart to apply</StatusBadge>
                ) : null}
              </div>
            </SettingRow>
            <SettingRow
              label="ZIP export"
              htmlFor="module-zip-export"
              help="Download a filtered set of files from a collection as one ZIP (_zip_exports). A change needs a server restart, same as Queue."
            >
              <div className="flex items-center gap-2">
                <ToggleSetting
                  id="module-zip-export"
                  checked={draft.zipExport.enabled}
                  onChange={(enabled) => patch({ zipExport: { enabled } })}
                  label={draft.zipExport.enabled ? "Enabled" : "Disabled"}
                />
                {draft.zipExport.enabled !== settings.zipExport.enabled ? (
                  <StatusBadge tone="warning">Restart to apply</StatusBadge>
                ) : null}
              </div>
            </SettingRow>
          </SettingsSection>

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
              error={fieldErrs.batchMax}
            >
              <NumberSetting
                id="batch-max"
                min={1}
                value={draft.batch.maxRequests}
                onChange={(maxRequests) => patch({ batch: { ...draft.batch, maxRequests } })}
                suffix="operations"
                invalid={!!fieldErrs.batchMax}
              />
            </SettingRow>
            <SettingRow
              label="Timeout"
              htmlFor="batch-timeout"
              help="A batch that runs longer than this is rolled back."
              error={fieldErrs.batchTimeout}
            >
              <NumberSetting
                id="batch-timeout"
                min={1}
                value={draft.batch.timeout}
                onChange={(timeout) => patch({ batch: { ...draft.batch, timeout } })}
                suffix="seconds"
                invalid={!!fieldErrs.batchTimeout}
              />
            </SettingRow>
            <SettingRow
              label="Max body size"
              htmlFor="batch-max-body"
              help="Reject a batch request whose raw body is larger than this, in bytes. 0 means no limit beyond the server's own request-size cap."
              error={fieldErrs.batchBody}
            >
              <NumberSetting
                id="batch-max-body"
                min={0}
                value={draft.batch.maxBodySize}
                onChange={(maxBodySize) => patch({ batch: { ...draft.batch, maxBodySize } })}
                suffix="bytes"
                invalid={!!fieldErrs.batchBody}
              />
            </SettingRow>
          </SettingsSection>
        </TabsContent>

        <TabsContent value="storage" className="flex flex-col gap-4 pt-2">
          <SettingsSection
            title="File storage"
            description="Where uploaded files live. Off keeps them on the server's own disk under the data directory."
            action={
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="shrink-0"
                disabled={testS3.isPending || !settings.s3.enabled || s3Edited}
                title={
                  !settings.s3.enabled
                    ? "Enable and save S3 first"
                    : s3Edited
                      ? "Save your S3 changes first — the test uses the saved settings"
                      : undefined
                }
                onClick={() => testS3.mutate("storage")}
              >
                {testS3.isPending ? <Spinner /> : <TestTube />}
                Test connection
              </Button>
            }
          >
            <SettingRow label="Use S3" htmlFor="s3-enabled" help="Off keeps uploads on the local disk; existing files are not moved when you switch.">
              <ToggleSetting
                id="s3-enabled"
                checked={draft.s3.enabled}
                onChange={(enabled) => patch({ s3: { ...draft.s3, enabled } })}
                label={draft.s3.enabled ? "Uploads go to S3" : "Uploads stay on local disk"}
              />
            </SettingRow>
            <SettingRow label="Bucket" htmlFor="s3-bucket" error={fieldErrs.s3Bucket}>
              <TextSetting
                id="s3-bucket"
                value={draft.s3.bucket}
                onChange={(bucket) => patch({ s3: { ...draft.s3, bucket } })}
                mono
                invalid={!!fieldErrs.s3Bucket}
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
            <SettingRow
              label="Endpoint"
              htmlFor="s3-endpoint"
              help="Any S3-compatible endpoint — R2, MinIO, Spaces."
              error={fieldErrs.s3Endpoint}
            >
              <TextSetting
                id="s3-endpoint"
                value={draft.s3.endpoint}
                onChange={(endpoint) => patch({ s3: { ...draft.s3, endpoint } })}
                placeholder="https://s3.amazonaws.com"
                mono
                invalid={!!fieldErrs.s3Endpoint}
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
              error={fieldErrs.maxDimension}
            >
              <NumberSetting
                id="storage-max-dimension"
                min={0}
                suffix="px (0 = no limit)"
                value={draft.storage.maxTransformDimension}
                onChange={(maxTransformDimension) =>
                  patch({ storage: { ...draft.storage, maxTransformDimension } })
                }
                invalid={!!fieldErrs.maxDimension}
              />
            </SettingRow>
            <SettingRow
              label="Per-user storage quota"
              htmlFor="storage-quota"
              help="Caps the total bytes a single auth record may store across every collection with an ownerField set (see that collection's settings). 0 disables the quota."
              error={fieldErrs.quota}
            >
              <NumberSetting
                id="storage-quota"
                min={0}
                suffix="bytes (0 = unlimited)"
                value={draft.storage.userQuotaBytes}
                onChange={(userQuotaBytes) => patch({ storage: { ...draft.storage, userQuotaBytes } })}
                invalid={!!fieldErrs.quota}
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
    </SettingsPage>
  );
}
