import { settingsAutomationRoute, type AutomationTab } from "@/routes/settings-automation";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { SettingsTabsPage } from "@/components/settings/settings-form";
import { CronJobsPage } from "@/components/settings/cron-jobs-page";
import { WebhooksPage } from "@/components/settings/webhooks-page";
import { FunctionsPage } from "@/components/settings/functions-page";
import { RealtimeChannelsPage } from "@/components/settings/realtime-channels-page";

const GROUP = SETTINGS_GROUPS.find((g) => g.to === "/settings/automation")!;

const COMPONENT_FOR: Record<AutomationTab, React.ReactNode> = {
  cron: <CronJobsPage />,
  webhooks: <WebhooksPage />,
  functions: <FunctionsPage />,
  realtime: <RealtimeChannelsPage />,
};

/** `/settings/automation` — the consolidated Automation settings group. */
export function AutomationSettingsPage() {
  const search = settingsAutomationRoute.useSearch();
  const navigate = settingsAutomationRoute.useNavigate();
  const tabs = GROUP.tabs.map((t) => ({ value: t.value, label: t.label, content: COMPONENT_FOR[t.value as AutomationTab] }));

  return (
    <SettingsTabsPage
      group={GROUP}
      tabs={tabs}
      tab={search.tab ?? "cron"}
      onTabChange={(tab) => void navigate({ search: { tab: tab as AutomationTab } })}
    />
  );
}
