import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where this page now lives — the "llm" tab of the consolidated
 * settings group at `/settings/integrations`. */
export const settingsLlmRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/llm",
  beforeLoad: () => {
    throw redirect({ to: "/settings/integrations", search: { tab: "llm" } });
  },
});
