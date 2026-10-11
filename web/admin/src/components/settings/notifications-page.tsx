import { useState } from "react";
import { toast } from "sonner";
import { describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { StatusBadge } from "@/components/layout/stat-card";
import { Button } from "@/components/ui/button";
import { NumberSetting, SettingRow, SettingsPage, SettingsSaveBar, SettingsSection } from "@/components/settings/settings-form";

const RETENTION_PRESETS = [
  { days: 7, label: "7 days" },
  { days: 30, label: "30 days" },
  { days: 90, label: "90 days" },
  { days: 0, label: "Forever" },
];

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
          description="Controls how long in-app notifications are kept. Email and push deliveries have their own logs (Mail log, provider dashboards)."
          action={
            <StatusBadge tone={current.retentionDays > 0 ? "info" : "neutral"}>
              {current.retentionDays > 0 ? `Pruned after ${current.retentionDays} days` : "Kept forever"}
            </StatusBadge>
          }
        >
          <SettingRow
            label="Delete read notifications after"
            htmlFor="notifications-retention"
            help="Only notifications the recipient has already marked read are ever deleted — unread ones are kept regardless of age. 0 keeps everything forever."
            error={errors[0]}
          >
            <div className="flex flex-col gap-2">
              <NumberSetting
                id="notifications-retention"
                min={0}
                value={current.retentionDays}
                onChange={(retentionDays) => setDraft({ retentionDays })}
                suffix="days"
                invalid={errors.length > 0}
              />
              <div role="group" aria-label="Retention presets" className="flex flex-wrap gap-1.5">
                {RETENTION_PRESETS.map((preset) => (
                  <Button
                    key={preset.days}
                    type="button"
                    size="xs"
                    variant={current.retentionDays === preset.days ? "secondary" : "outline"}
                    aria-pressed={current.retentionDays === preset.days}
                    onClick={() => setDraft({ retentionDays: preset.days })}
                  >
                    {preset.label}
                  </Button>
                ))}
              </div>
            </div>
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
