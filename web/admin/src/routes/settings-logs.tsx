import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type RequestLogsSearch = {
  page?: number;
  filter?: string;
};

export const settingsLogsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/logs",
  validateSearch: (search: Record<string, unknown>): RequestLogsSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    filter: typeof search.filter === "string" ? search.filter : undefined,
  }),
  // Lazy: this tab pulls in recharts for the request-volume chart.
  component: lazyRouteComponent(() => import("@/components/settings/request-logs-page"), "RequestLogsPage"),
});
