import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "templates" tab of the consolidated
 * settings group at `/settings/email`. */
export const settingsEmailTemplatesRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/email-templates",
  beforeLoad: () => {
    throw redirect({ to: "/settings/email", search: { tab: "templates" } });
  },
});
