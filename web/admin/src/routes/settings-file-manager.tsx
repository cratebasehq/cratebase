import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsFileManagerRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/file-manager",
  component: lazyRouteComponent(() => import("@/components/settings/file-manager-page"), "FileManagerPage"),
});
