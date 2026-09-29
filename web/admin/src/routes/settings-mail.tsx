import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** Old top-level route, kept only to redirect a bookmarked/shared link to
 * where the SMTP half of this page now lives — the "delivery" tab of the
 * consolidated settings group at `/settings/email`. The other half (S3
 * file storage and its image-transform/quota limits) has since moved to
 * the "storage" tab of `/settings/application` — see
 * `settingsApplicationRoute` — since it isn't an email concern. */
export const settingsMailRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-storage",
  beforeLoad: () => {
    throw redirect({ to: "/settings/email", search: { tab: "delivery" } });
  },
});
