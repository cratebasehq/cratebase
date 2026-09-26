import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsEmailTemplatesRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/email-templates",
  component: lazyRouteComponent(() => import("@/components/settings/email-templates-page"), "EmailTemplatesPage"),
});
