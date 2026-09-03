import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { RequestLogsPage } from "@/components/settings/request-logs-page";

export const settingsLogsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/logs",
  component: RequestLogsPage,
});
