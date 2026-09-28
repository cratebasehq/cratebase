import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type DatabaseTab = "sql" | "extensions" | "rpc" | "backups" | "files";

export type DatabaseSearch = { tab?: DatabaseTab };

function isDatabaseTab(value: unknown): value is DatabaseTab {
  return value === "sql" || value === "extensions" || value === "rpc" || value === "backups" || value === "files";
}

/** `/settings/database` — the "Database" settings group: SQL console,
 * Extensions, RPC functions, Backups, and File manager as tabs. */
export const settingsDatabaseRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/database",
  validateSearch: (search: Record<string, unknown>): DatabaseSearch => ({
    tab: isDatabaseTab(search.tab) ? search.tab : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/database-settings-page"), "DatabaseSettingsPage"),
});
