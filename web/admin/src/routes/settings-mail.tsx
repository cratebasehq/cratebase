import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsMailRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-storage",
  component: lazyRouteComponent(() => import("@/components/settings/mail-storage-page"), "MailStoragePage"),
});
