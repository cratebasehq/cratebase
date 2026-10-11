import { useRouterState } from "@tanstack/react-router";

/** The current pathname and `?tab=` value — everything nav highlighting and
 * breadcrumbs need, with the tab narrowed to a string. */
export function useNavLocation(): { pathname: string; tab: string | undefined } {
  return useRouterState({
    select: (state) => {
      const search: Record<string, unknown> = state.location.search;
      return {
        pathname: state.location.pathname,
        tab: typeof search.tab === "string" ? search.tab : undefined,
      };
    },
  });
}
