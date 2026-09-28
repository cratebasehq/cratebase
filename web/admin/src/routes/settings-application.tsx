import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type ApplicationTab = "general" | "branding" | "modules";

export type ApplicationSearch = { tab?: ApplicationTab };

function isApplicationTab(value: unknown): value is ApplicationTab {
  return value === "general" || value === "branding" || value === "modules";
}

/**
 * `/settings/application` — the "Application" settings group: General,
 * Branding, and Modules tabs (`?tab=`), consolidated from what used to be
 * a single flat page. Kept as one route/one page component (not three)
 * because all three tabs edit slices of the same settings object and
 * share one draft/save bar — see `ApplicationPage`.
 */
export const settingsApplicationRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/application",
  validateSearch: (search: Record<string, unknown>): ApplicationSearch => ({
    tab: isApplicationTab(search.tab) ? search.tab : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/application-page"), "ApplicationPage"),
});
