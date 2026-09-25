import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsEmailTriggersRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/email-triggers",
  component: lazyRouteComponent(() => import("@/components/settings/email-triggers-page"), "EmailTriggersPage"),
});
