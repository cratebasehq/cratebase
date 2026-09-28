import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type MailInboxSearch = {
  id?: string;
};

/** Old top-level route, kept only to redirect a bookmarked/shared link
 * (including a deep link to one specific message, `?id=`) to the "Dev
 * inbox" tab of the consolidated Email settings group. */
export const settingsMailInboxRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-inbox",
  validateSearch: (search: Record<string, unknown>): MailInboxSearch => ({
    id: typeof search.id === "string" && search.id.length > 0 ? search.id : undefined,
  }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/settings/email", search: { tab: "dev-inbox", id: search.id } });
  },
});
