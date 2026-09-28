import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsRpcRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/rpc",
  // Lazy for the same reason as the SQL console: CodeMirror + its SQL
  // language package are heavy, and this tab is opened rarely.
  component: lazyRouteComponent(() => import("@/components/settings/rpc-functions-page"), "RpcFunctionsPage"),
});
