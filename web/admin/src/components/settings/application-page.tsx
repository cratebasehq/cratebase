import { useMemo, useState } from "react";
import { toast } from "sonner";
import { describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsApplicationRoute, type ApplicationTab } from "@/routes/settings-application";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  NumberSetting,
  SettingRow,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

/** The slice of settings this page owns. Kept as one draft across all
 * three tabs (General/Branding/Modules) — they're small enough, and share
 * a save bar, that splitting the draft per-tab would only add friction
 * (switching tabs with unsaved edits would need to warn, or lose them). */
type Draft = Pick<ServerSettings, "meta" | "batch" | "teams" | "queue" | "zipExport">;

function draftOf(settings: ServerSettings): Draft {
  return {
    meta: { ...settings.meta },
    batch: { ...settings.batch },
    teams: { ...settings.teams },
    queue: { ...settings.queue },
    zipExport: { ...settings.zipExport },
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
  return errors;
}

function dirtyAgainst(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

export function ApplicationPage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
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
    save.mutate(
      { meta: draft.meta, batch: draft.batch, teams: draft.teams, queue: draft.queue, zipExport: draft.zipExport },
      {
        onSuccess: () => toast.success("Settings saved"),
        onError: (error) => {
          const failure = describeFailure(error);
          const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
          toast.error(failure.title, {
            description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
          });
        },
      },
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page pb-24">
      <div className="flex flex-col">
        <h1 className="text-base font-medium tracking-tight">Application</h1>
        <p className="mt-0.5 max-w-measure text-sm text-muted-foreground">
          How this instance identifies itself, its branding, and which optional modules are on.
        </p>
      </div>

      <Tabs value={tab} onValueChange={(next) => void navigate({ search: { tab: next as ApplicationTab } })}>
        <TabsList>
          <TabsTrigger value="general">General</TabsTrigger>
          <TabsTrigger value="branding">Branding</TabsTrigger>
          <TabsTrigger value="modules">Modules</TabsTrigger>
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
