import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "network" tab of the consolidated
 * settings group at `/settings/auth`. */
export const settingsNetworkRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/network",
  beforeLoad: () => {
    throw redirect({ to: "/settings/auth", search: { tab: "network" } });
  },
});
