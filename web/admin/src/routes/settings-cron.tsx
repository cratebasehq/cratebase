import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { CronJobsPage } from "@/components/settings/cron-jobs-page";

export const settingsCronRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/cron",
  component: CronJobsPage,
});
