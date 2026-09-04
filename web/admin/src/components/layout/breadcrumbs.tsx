import { Fragment } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";

type Crumb = { label: string; to?: string; mono?: boolean };

const SETTINGS_TABS: Record<string, string> = {
  logs: "Request logs",
  backups: "Backups",
  cron: "Scheduled jobs",
};

/**
 * Breadcrumbs are derived from the pathname rather than from route static
 * data: the router is code-based and there are six screens, so a single
 * readable mapping beats threading a `crumb` through every route definition.
 */
function crumbsFor(pathname: string): Crumb[] {
  const segments = pathname.split("/").filter(Boolean);

  if (segments[0] === "collections") {
    const name = segments[1];
    if (!name) return [{ label: "Collections" }];
    return [{ label: "Collections" }, { label: decodeURIComponent(name), mono: true }];
  }

  if (segments[0] === "settings") {
    const tab = segments[1];
    const crumbs: Crumb[] = [{ label: "Settings", to: "/settings/logs" }];
    if (tab && SETTINGS_TABS[tab]) crumbs.push({ label: SETTINGS_TABS[tab] });
    return crumbs;
  }

  return [{ label: "Overview" }];
}

export function Breadcrumbs() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const crumbs = crumbsFor(pathname);

  return (
    <Breadcrumb>
      <BreadcrumbList className="gap-1 text-sm sm:gap-1">
        {crumbs.map((crumb, index) => {
          const last = index === crumbs.length - 1;
          return (
            <Fragment key={`${crumb.label}-${index}`}>
              <BreadcrumbItem>
                {last ? (
                  <BreadcrumbPage className={crumb.mono ? "font-mono" : undefined}>
                    {crumb.label}
                  </BreadcrumbPage>
                ) : crumb.to ? (
                  <BreadcrumbLink asChild>
                    <Link to={crumb.to}>{crumb.label}</Link>
                  </BreadcrumbLink>
                ) : (
                  <span className="text-muted-foreground">{crumb.label}</span>
                )}
              </BreadcrumbItem>
              {last ? null : <BreadcrumbSeparator />}
            </Fragment>
          );
        })}
      </BreadcrumbList>
    </Breadcrumb>
  );
}
