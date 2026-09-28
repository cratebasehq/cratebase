import { settingsAuthRoute, type AuthSecurityTab } from "@/routes/settings-auth";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { SettingsTabsPage } from "@/components/settings/settings-form";
import { SuperusersPage } from "@/components/settings/superusers-page";
import { SessionsPage } from "@/components/settings/sessions-page";
import { ApiKeysPage } from "@/components/settings/api-keys-page";
import { NetworkPage } from "@/components/settings/network-page";

const GROUP = SETTINGS_GROUPS.find((g) => g.to === "/settings/auth")!;

const COMPONENT_FOR: Record<AuthSecurityTab, React.ReactNode> = {
  superusers: <SuperusersPage />,
  sessions: <SessionsPage />,
  "api-keys": <ApiKeysPage />,
  network: <NetworkPage />,
};

/** `/settings/auth` — the consolidated Auth & security settings group. */
export function AuthSecuritySettingsPage() {
  const search = settingsAuthRoute.useSearch();
  const navigate = settingsAuthRoute.useNavigate();
  const tabs = GROUP.tabs.map((t) => ({ value: t.value, label: t.label, content: COMPONENT_FOR[t.value as AuthSecurityTab] }));

  return (
    <SettingsTabsPage
      group={GROUP}
      tabs={tabs}
      tab={search.tab ?? "superusers"}
      onTabChange={(tab) => void navigate({ search: { tab: tab as AuthSecurityTab } })}
    />
  );
}
