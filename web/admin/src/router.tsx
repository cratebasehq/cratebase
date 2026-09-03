import { createRouter } from "@tanstack/react-router";
import { rootRoute } from "@/routes/root";
import { loginRoute } from "@/routes/login";
import { appRoute } from "@/routes/app";
import { dashboardIndexRoute } from "@/routes/dashboard-index";
import { collectionRoute } from "@/routes/collection";

const routeTree = rootRoute.addChildren([
  loginRoute,
  appRoute.addChildren([dashboardIndexRoute, collectionRoute]),
]);

export const router = createRouter({ routeTree, defaultPreload: "intent" });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
