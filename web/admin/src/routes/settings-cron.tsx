import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "cron" tab of the consolidated
 * settings group at `/settings/automation`. */
export const settingsCronRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/cron",
  beforeLoad: () => {
    throw redirect({ to: "/settings/automation", search: { tab: "cron" } });
  },
});
