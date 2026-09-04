import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsCronRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/cron",
  component: lazyRouteComponent(() => import("@/components/settings/cron-jobs-page"), "CronJobsPage"),
});
