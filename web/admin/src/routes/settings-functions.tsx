import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "functions" tab of the consolidated
 * settings group at `/settings/automation`. */
export const settingsFunctionsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/functions",
  beforeLoad: () => {
    throw redirect({ to: "/settings/automation", search: { tab: "functions" } });
  },
});
