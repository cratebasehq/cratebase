import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsSessionsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/sessions",
  component: lazyRouteComponent(() => import("@/components/settings/sessions-page"), "SessionsPage"),
});
