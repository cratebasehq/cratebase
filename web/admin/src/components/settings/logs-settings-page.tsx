import { settingsLogsRoute, type LogsTab } from "@/routes/settings-logs";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { SettingsTabsPage } from "@/components/settings/settings-form";
import { RequestLogsPage } from "@/components/settings/request-logs-page";
import { AuditLogPage } from "@/components/settings/audit-log-page";

const GROUP = SETTINGS_GROUPS.find((g) => g.to === "/settings/logs")!;

const COMPONENT_FOR: Record<LogsTab, React.ReactNode> = {
  requests: <RequestLogsPage />,
  audit: <AuditLogPage />,
};

/** `/settings/logs` — the consolidated Logs settings group: Request logs
 * and Audit log as tabs. Both tabs share the `page` search key (only one
 * is ever showing), so switching tabs resets it. */
export function LogsSettingsPage() {
  const search = settingsLogsRoute.useSearch();
  const navigate = settingsLogsRoute.useNavigate();
  const tabs = GROUP.tabs.map((t) => ({ value: t.value, label: t.label, content: COMPONENT_FOR[t.value as LogsTab] }));

  return (
    <SettingsTabsPage
      group={GROUP}
      tabs={tabs}
      tab={search.tab ?? "requests"}
      onTabChange={(tab) => void navigate({ search: { tab: tab as LogsTab } })}
    />
  );
}
