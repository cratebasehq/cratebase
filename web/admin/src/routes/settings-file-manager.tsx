import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "files" tab of the consolidated
 * settings group at `/settings/database`. */
export const settingsFileManagerRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/file-manager",
  beforeLoad: () => {
    throw redirect({ to: "/settings/database", search: { tab: "files" } });
  },
});
