import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { ApplicationPage } from "@/components/settings/application-page";

export const settingsApplicationRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/application",
  component: ApplicationPage,
});
