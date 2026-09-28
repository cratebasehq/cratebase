import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "webhooks" tab of the consolidated
 * settings group at `/settings/automation`. */
export const settingsWebhooksRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/webhooks",
  beforeLoad: () => {
    throw redirect({ to: "/settings/automation", search: { tab: "webhooks" } });
  },
});
