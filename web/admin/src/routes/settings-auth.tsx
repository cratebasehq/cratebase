import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type AuthSecurityTab = "superusers" | "sessions" | "api-keys" | "network";

export type AuthSecuritySearch = { tab?: AuthSecurityTab };

function isAuthSecurityTab(value: unknown): value is AuthSecurityTab {
  return value === "superusers" || value === "sessions" || value === "api-keys" || value === "network";
}

/** `/settings/auth` — the "Auth & security" settings group: Superusers,
 * Sessions, API keys, and Network as tabs. */
export const settingsAuthRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/auth",
  validateSearch: (search: Record<string, unknown>): AuthSecuritySearch => ({
    tab: isAuthSecurityTab(search.tab) ? search.tab : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/auth-security-settings-page"), "AuthSecuritySettingsPage"),
});
