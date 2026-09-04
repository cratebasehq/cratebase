import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { NetworkPage } from "@/components/settings/network-page";

export const settingsNetworkRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/network",
  component: NetworkPage,
});
