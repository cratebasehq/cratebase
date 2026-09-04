import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { AnalyticsPage } from "@/components/settings/analytics-page";

export type AnalyticsSearch = {
  page?: number;
};

export const settingsAnalyticsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/analytics",
  validateSearch: (search: Record<string, unknown>): AnalyticsSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
  }),
  component: AnalyticsPage,
});
