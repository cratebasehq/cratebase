import { settingsEmailRoute, type EmailTab } from "@/routes/settings-email";
import { SETTINGS_GROUPS, isSettingsTabVisible } from "@/lib/settings-nav";
import { useDevMailInboxAvailable, useSettings } from "@/hooks/use-settings";
import { SettingsTabsPage } from "@/components/settings/settings-form";
import { MailStoragePage } from "@/components/settings/mail-storage-page";
import { EmailTemplatesPage } from "@/components/settings/email-templates-page";
import { EmailTriggersPage } from "@/components/settings/email-triggers-page";
import { MailLogPage } from "@/components/settings/mail-log-page";
import { MailInboxPage } from "@/components/settings/mail-inbox-page";

const GROUP = SETTINGS_GROUPS.find((g) => g.to === "/settings/email")!;

/** `/settings/email` — the consolidated Email settings group: Delivery,
 * Templates, Triggers, Mail log, and Dev inbox (while the zero-config Log
 * backend is active) as tabs of one page. */
export function EmailSettingsPage() {
  const search = settingsEmailRoute.useSearch();
  const navigate = settingsEmailRoute.useNavigate();
  const { data: settings } = useSettings();
  const { data: devMailInboxAvailable } = useDevMailInboxAvailable();

  const componentFor: Record<EmailTab, React.ReactNode> = {
    delivery: <MailStoragePage />,
    templates: <EmailTemplatesPage />,
    triggers: <EmailTriggersPage />,
    "mail-log": <MailLogPage />,
    "dev-inbox": <MailInboxPage />,
  };

  const tabs = GROUP.tabs
    .filter((t) => isSettingsTabVisible(t.legacyPath, settings, devMailInboxAvailable))
    .map((t) => ({ value: t.value, label: t.label, content: componentFor[t.value as EmailTab] }));

  return (
    <SettingsTabsPage
      group={GROUP}
      tabs={tabs}
      tab={search.tab ?? "delivery"}
      onTabChange={(tab) => void navigate({ search: (prev) => ({ ...prev, tab: tab as EmailTab, page: undefined }) })}
    />
  );
}
