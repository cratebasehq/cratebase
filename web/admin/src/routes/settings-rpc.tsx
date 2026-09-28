import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "rpc" tab of the consolidated
 * settings group at `/settings/database`. */
export const settingsRpcRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/rpc",
  beforeLoad: () => {
    throw redirect({ to: "/settings/database", search: { tab: "rpc" } });
  },
});
