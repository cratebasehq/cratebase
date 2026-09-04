import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsSqlConsoleRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/sql",
  // Lazy: CodeMirror + its SQL language package are the single heaviest
  // dependency in the settings area, and this tab is opened rarely.
  component: lazyRouteComponent(() => import("@/components/settings/sql-console-page"), "SqlConsolePage"),
});
