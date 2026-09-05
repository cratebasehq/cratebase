import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type AnalyticsSearch = {
  page?: number;
};

export const settingsAnalyticsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/analytics",
  validateSearch: (search: Record<string, unknown>): AnalyticsSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
  }),
  // Lazy: pulls in recharts, which is heavy enough to keep out of the
  // main bundle for a tab most operators only open occasionally.
  component: lazyRouteComponent(() => import("@/components/settings/analytics-page"), "AnalyticsPage"),
});
