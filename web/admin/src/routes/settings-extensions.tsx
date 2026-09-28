import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "extensions" tab of the consolidated
 * settings group at `/settings/database`. */
export const settingsExtensionsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/extensions",
  beforeLoad: () => {
    throw redirect({ to: "/settings/database", search: { tab: "extensions" } });
  },
});
