import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "delivery" tab of the consolidated
 * settings group at `/settings/email`. */
export const settingsMailRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-storage",
  beforeLoad: () => {
    throw redirect({ to: "/settings/email", search: { tab: "delivery" } });
  },
});
