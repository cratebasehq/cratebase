import { settingsDatabaseRoute, type DatabaseTab } from "@/routes/settings-database";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { SettingsTabsPage } from "@/components/settings/settings-form";
import { SqlConsolePage } from "@/components/settings/sql-console-page";
import { DbExtensionsPage } from "@/components/settings/db-extensions-page";
import { RpcFunctionsPage } from "@/components/settings/rpc-functions-page";
import { BackupsPage } from "@/components/settings/backups-page";
import { FileManagerPage } from "@/components/settings/file-manager-page";

const GROUP = SETTINGS_GROUPS.find((g) => g.to === "/settings/database")!;

const COMPONENT_FOR: Record<DatabaseTab, React.ReactNode> = {
  sql: <SqlConsolePage />,
  extensions: <DbExtensionsPage />,
  rpc: <RpcFunctionsPage />,
  backups: <BackupsPage />,
  files: <FileManagerPage />,
};

/** `/settings/database` — the consolidated Database settings group. */
export function DatabaseSettingsPage() {
  const search = settingsDatabaseRoute.useSearch();
  const navigate = settingsDatabaseRoute.useNavigate();
  const tabs = GROUP.tabs.map((t) => ({ value: t.value, label: t.label, content: COMPONENT_FOR[t.value as DatabaseTab] }));

  return (
    <SettingsTabsPage
      group={GROUP}
      tabs={tabs}
      tab={search.tab ?? "sql"}
      onTabChange={(tab) => void navigate({ search: { tab: tab as DatabaseTab } })}
    />
  );
}
