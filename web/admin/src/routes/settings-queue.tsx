import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Bookmark-compatibility route, kept only to redirect to where this page
 * lives — the "queue" tab of the consolidated settings group at
 * `/settings/automation`. */
export const settingsQueueRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/queue",
  beforeLoad: () => {
    throw redirect({ to: "/settings/automation", search: { tab: "queue" } });
  },
});
