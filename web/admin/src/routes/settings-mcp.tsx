import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "mcp" tab of the consolidated
 * settings group at `/settings/integrations`. */
export const settingsMcpRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mcp",
  beforeLoad: () => {
    throw redirect({ to: "/settings/integrations", search: { tab: "mcp" } });
  },
});
