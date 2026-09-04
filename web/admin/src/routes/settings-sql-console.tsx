import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { SqlConsolePage } from "@/components/settings/sql-console-page";

export const settingsSqlConsoleRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/sql",
  component: SqlConsolePage,
});
