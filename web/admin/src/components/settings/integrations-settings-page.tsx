import { settingsIntegrationsRoute, type IntegrationsTab } from "@/routes/settings-integrations";
import { SETTINGS_GROUPS, isSettingsTabVisible } from "@/lib/settings-nav";
import { useSettings } from "@/hooks/use-settings";
import { SettingsTabsPage } from "@/components/settings/settings-form";
import { PushPage } from "@/components/settings/push-page";
import { LlmPage } from "@/components/settings/llm-page";
import { McpPage } from "@/components/settings/mcp-page";
import { NotificationsPage } from "@/components/settings/notifications-page";

const GROUP = SETTINGS_GROUPS.find((g) => g.to === "/settings/integrations")!;

const COMPONENT_FOR: Record<IntegrationsTab, React.ReactNode> = {
  push: <PushPage />,
  llm: <LlmPage />,
  mcp: <McpPage />,
  notifications: <NotificationsPage />,
};

/** `/settings/integrations` — the consolidated Integrations settings
 * group: Push, LLM (only once enabled), and MCP as tabs. */
export function IntegrationsSettingsPage() {
  const search = settingsIntegrationsRoute.useSearch();
  const navigate = settingsIntegrationsRoute.useNavigate();
  const { data: settings } = useSettings();

  const tabs = GROUP.tabs
    .filter((t) => isSettingsTabVisible(t.legacyPath, settings))
    .map((t) => ({ value: t.value, label: t.label, content: COMPONENT_FOR[t.value as IntegrationsTab] }));

  return (
    <SettingsTabsPage
      group={GROUP}
      tabs={tabs}
      tab={search.tab ?? "push"}
      onTabChange={(tab) => void navigate({ search: { tab: tab as IntegrationsTab } })}
    />
  );
}
