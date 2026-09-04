import { createRouter } from "@tanstack/react-router";
import { RouteError, RouteNotFound, RoutePending } from "@/components/app-states";
import { rootRoute } from "@/routes/root";
import { loginRoute } from "@/routes/login";
import { appRoute } from "@/routes/app";
import { dashboardIndexRoute } from "@/routes/dashboard-index";
import { collectionRoute } from "@/routes/collection";
import { settingsRoute } from "@/routes/settings";
import { settingsIndexRoute } from "@/routes/settings-index";
import { settingsLogsRoute } from "@/routes/settings-logs";
import { settingsBackupsRoute } from "@/routes/settings-backups";
import { settingsCronRoute } from "@/routes/settings-cron";

const routeTree = rootRoute.addChildren([
  loginRoute,
  appRoute.addChildren([
    dashboardIndexRoute,
    collectionRoute,
    settingsRoute.addChildren([settingsIndexRoute, settingsLogsRoute, settingsBackupsRoute, settingsCronRoute]),
  ]),
]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  // Every route now has a designed failure, loading and 404 state. Before
  // this, a throw in a loader rendered nothing and an unknown URL rendered
  // the router's built-in placeholder.
  defaultErrorComponent: RouteError,
  defaultPendingComponent: RoutePending,
  defaultNotFoundComponent: RouteNotFound,
  // Long enough that a fast local backend never flashes a skeleton.
  defaultPendingMs: 250,
  defaultPendingMinMs: 320,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
