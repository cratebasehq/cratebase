import { useState } from "react";
import { toast } from "sonner";
import { describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { NumberSetting, SettingRow, SettingsPage, SettingsSaveBar, SettingsSection } from "@/components/settings/settings-form";

type NotificationsSettings = ServerSettings["notifications"];

function draftOf(settings: ServerSettings): NotificationsSettings {
  return { ...settings.notifications };
}

/**
 * `/settings/integrations?tab=notifications` — retention for `_notifications`
 * rows. Sending is not configured here: `$notify.send`/
 * `POST /api/notifications/send` always create the row and best-effort
 * deliver `email`/`push` immediately, with no setting to turn that off
 * globally (a caller narrows `channels` per call instead) — this page only
 * controls how long a *read* row is kept afterward.
 */
export function NotificationsPage() {
  const { data: settings } = useSettings();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<NotificationsSettings | null>(null);

  const current = draft ?? (settings ? draftOf(settings) : null);
  const dirty = Boolean(draft && settings && draft.retentionDays !== settings.notifications.retentionDays);
  const errors = current && current.retentionDays < 0 ? ["Retention can't be negative"] : [];

  const item = settingsItemFor("/settings/notifications")!;

  function submit() {
    if (!current || errors.length > 0) return;
    save.mutate(
      { notifications: current },
      {
        onSuccess: () => {
          toast.success("Notification settings saved");
          setDraft(null);
        },
        onError: (error) => {
          const failed = describeFailure(error);
          toast.error(failed.title, { description: failed.detail || failed.serverMessage || undefined });
        },
      },
    );
  }

  return (
    <SettingsPage title={item.label} description={item.description} width="form">
      {current ? (
        <SettingsSection
          title="Retention"
          description="Applies to _notifications rows only — email/push deliveries have their own logs (Mail log, provider dashboards)."
        >
          <SettingRow
            label="Delete read notifications after"
            htmlFor="notifications-retention"
            help="Only a notification the recipient has already marked read is ever deleted — an unread one is kept regardless of age. 0 keeps them forever."
            error={errors[0]}
          >
            <NumberSetting
              id="notifications-retention"
              min={0}
              value={current.retentionDays}
              onChange={(retentionDays) => setDraft({ retentionDays })}
              suffix="days"
            />
          </SettingRow>
        </SettingsSection>
      ) : null}
      {current ? (
        <SettingsSaveBar
          dirty={dirty}
          pending={save.isPending}
          errors={errors}
          onSave={submit}
          onReset={() => setDraft(null)}
        />
      ) : null}
    </SettingsPage>
  );
}
