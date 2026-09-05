import { useMemo, useState } from "react";
import { toast } from "sonner";
import { describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { Skeleton } from "@/components/ui/skeleton";
import {
  NumberSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

/** The slice of settings this page owns. Keeping it explicit is what lets
 * the save send only these keys, so two people editing different pages
 * don't overwrite each other's sections. */
type Draft = Pick<ServerSettings, "meta" | "batch">;

function draftOf(settings: ServerSettings): Draft {
  return { meta: { ...settings.meta }, batch: { ...settings.batch } };
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
  if (draft.batch.maxRequests < 1) errors.push("A batch must allow at least one request");
  if (draft.batch.timeout < 1) errors.push("Batch timeout must be at least 1 second");
  return errors;
}

export function ApplicationPage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(settings);

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
      { meta: draft.meta, batch: draft.batch },
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

  const item = settingsItemFor("/settings/application")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="form">
      <SettingsSection
        title="Application"
        description="How this instance identifies itself in emails and links."
      >
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
      </SettingsSection>

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

function dirtyAgainst(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}
