import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsLlmRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/llm",
  component: lazyRouteComponent(() => import("@/components/settings/llm-page"), "LlmPage"),
});
