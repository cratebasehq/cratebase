import { createRoute, redirect } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type MailLogSearch = {
  page?: number;
  status?: "sent" | "failed" | "queued";
  template?: string;
};

/** Old top-level route, kept only to redirect a bookmarked/shared link
 * (filters and all) to the "Mail log" tab of the consolidated Email
 * settings group. */
export const settingsMailLogRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-log",
  validateSearch: (search: Record<string, unknown>): MailLogSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    status:
      search.status === "sent" || search.status === "failed" || search.status === "queued" ? search.status : undefined,
    template: typeof search.template === "string" && search.template.length > 0 ? search.template : undefined,
  }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/settings/email", search: { tab: "mail-log", ...search } });
  },
});
