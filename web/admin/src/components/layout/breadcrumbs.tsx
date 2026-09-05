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
import { settingsItemFor } from "@/lib/settings-nav";

type Crumb = { label: string; to?: string; mono?: boolean };

/**
 * Breadcrumbs are derived from the pathname rather than from route static
 * data: the router is code-based and there are six screens, so a single
 * readable mapping beats threading a `crumb` through every route definition.
 */
function crumbsFor(pathname: string, tab?: string): Crumb[] {
  const segments = pathname.split("/").filter(Boolean);

  if (segments[0] === "collections") {
    const name = segments[1];
    if (!name) return [{ label: "Collections" }];
    const crumbs: Crumb[] = [{ label: "Collections" }, { label: decodeURIComponent(name), mono: true }];
    if (tab === "schema") crumbs.push({ label: "Schema" });
    else if (tab === "api") crumbs.push({ label: "API" });
    return crumbs;
  }

  if (segments[0] === "settings") {
    const crumbs: Crumb[] = [{ label: "Settings", to: "/settings" }];
    const item = settingsItemFor(pathname);
    if (item) crumbs.push({ label: item.label });
    return crumbs;
  }

  return [{ label: "Overview" }];
}

export function Breadcrumbs() {
  const { pathname, tab } = useRouterState({
    select: (state) => ({
      pathname: state.location.pathname,
      tab: (state.location.search as { tab?: string }).tab,
    }),
  });
  const crumbs = crumbsFor(pathname, tab);

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
