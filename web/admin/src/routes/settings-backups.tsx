import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsBackupsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/backups",
  component: lazyRouteComponent(() => import("@/components/settings/backups-page"), "BackupsPage"),
});
