import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type ApplicationTab = "general" | "branding" | "modules" | "storage";

export type ApplicationSearch = { tab?: ApplicationTab };

function isApplicationTab(value: unknown): value is ApplicationTab {
  return value === "general" || value === "branding" || value === "modules" || value === "storage";
}

/**
 * `/settings/application` — the "Application" settings group: General,
 * Branding, Modules, and Storage tabs (`?tab=`), consolidated from what
 * used to be a single flat page. Kept as one route/one page component
 * (not four) because every tab edits a slice of the same settings object
 * and shares one draft/save bar — see `ApplicationPage`. Storage (S3 +
 * image-transform/quota limits) joined the group later, moved here from
 * Email → Delivery: it isn't an email concern, and living there made it
 * easy to miss.
 */
export const settingsApplicationRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/application",
  validateSearch: (search: Record<string, unknown>): ApplicationSearch => ({
    tab: isApplicationTab(search.tab) ? search.tab : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/application-page"), "ApplicationPage"),
});
