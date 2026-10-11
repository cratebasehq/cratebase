import { useMemo, useState } from "react";
import type { CollectionModel } from "@cratebase/client";
import { Link } from "@tanstack/react-router";
import { ChevronRight, Database, Download, Plus, ShieldUser, Upload } from "lucide-react";
import { CratebaseMark } from "@/components/brand/cratebase-mark";
import { AccountMenu } from "@/components/layout/account-menu";
import { useNavLocation } from "@/hooks/use-nav-location";
import { useSettings } from "@/hooks/use-settings";
import { useLocalState } from "@/hooks/use-local-state";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Kbd } from "@/components/ui/kbd";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarSeparator,
  useSidebar,
} from "@/components/ui/sidebar";
import { isNavItemActive, navLinkProps, NAV_GROUPS, OVERVIEW, type NavGroup, type NavItem } from "@/lib/nav";
import { cn } from "@/lib/utils";

/** A schema export is named `cb_schema.json` and ships the collection
 * array as-is (no envelope), so it re-imports through the import dialog
 * unchanged. */
function downloadCollectionsExport(collections: CollectionModel[]) {
  const blob = new Blob([JSON.stringify(collections, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "cb_schema.json";
  anchor.click();
  URL.revokeObjectURL(url);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((v) => typeof v === "string");
}

export function AppSidebar({
  collections,
  loading,
  onNewCollection,
  onImportCollections,
}: {
  collections: CollectionModel[];
  loading: boolean;
  onNewCollection: () => void;
  onImportCollections: () => void;
}) {
  const { pathname, tab } = useNavLocation();
  // Groups the user folded away. Stored as the *closed* set so a group added
  // later defaults to open.
  const [closed, setClosed] = useLocalState<string[]>("cb.nav.closed", [], isStringArray);

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader className="h-topbar justify-center border-b border-sidebar-border px-2">
        <Link
          to="/"
          aria-label="Cratebase overview"
          className="flex items-center gap-2 rounded-md px-1 py-1 outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring"
        >
          <CratebaseMark className="size-5 shrink-0" />
          <span className="text-sm font-semibold tracking-tight group-data-[collapsible=icon]:hidden">
            Cratebase
          </span>
        </Link>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup className="pb-0">
          <SidebarGroupContent>
            <SidebarMenu>
              <NavLink item={OVERVIEW} pathname={pathname} tab={tab} />
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        {NAV_GROUPS.map((group) => {
          const hasActive = group.items.some((item) => isNavItemActive(item, pathname, tab));
          const collectionActive = group.id === "data" && pathname.startsWith("/collections/");
          const open = hasActive || collectionActive || !closed.includes(group.id);
          return (
            <NavSection
              key={group.id}
              group={group}
              open={open}
              onOpenChange={(next) =>
                setClosed(next ? closed.filter((id) => id !== group.id) : [...closed, group.id])
              }
              pathname={pathname}
              tab={tab}
              before={
                group.id === "data" ? (
                  <CollectionsNav
                    collections={collections}
                    loading={loading}
                    pathname={pathname}
                    onNewCollection={onNewCollection}
                    onImportCollections={onImportCollections}
                  />
                ) : null
              }
            />
          );
        })}
      </SidebarContent>

      <SidebarSeparator className="mx-0" />
      <SidebarFooter>
        <div className="flex items-center justify-between gap-2 px-2 pt-1 text-2xs text-muted-foreground max-md:hidden group-data-[collapsible=icon]:hidden">
          <span className="flex items-center gap-1">
            <Kbd>⌘K</Kbd> search
          </span>
          <span className="flex items-center gap-1">
            <Kbd>⌘B</Kbd> sidebar
          </span>
        </div>
        <AccountMenu />
      </SidebarFooter>
    </Sidebar>
  );
}

/** One row of the nav. The chord hint (`G O`) only shows on hover/focus so it
 * teaches the shortcut without adding permanent noise. */
function NavLink({
  item,
  pathname,
  tab,
  badge,
}: {
  item: NavItem;
  pathname: string;
  tab: string | undefined;
  badge?: string;
}) {
  const Icon = item.icon;
  return (
    <SidebarMenuItem>
      <SidebarMenuButton asChild isActive={isNavItemActive(item, pathname, tab)} tooltip={item.label}>
        <Link {...(navLinkProps(item) as object as { to: string })}>
          <Icon />
          <span className="flex-1">{item.label}</span>
          {badge ? <span className="text-2xs text-muted-foreground group-data-[collapsible=icon]:hidden">{badge}</span> : null}
          {item.chord ? (
            <span
              aria-hidden
              className="hidden font-mono text-2xs tracking-widest text-muted-foreground/80 uppercase group-hover/menu-button:inline group-focus-visible/menu-button:inline group-data-[collapsible=icon]:hidden!"
            >
              g {item.chord}
            </span>
          ) : null}
        </Link>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

function NavSection({
  group,
  open,
  onOpenChange,
  pathname,
  tab,
  before,
}: {
  group: NavGroup;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pathname: string;
  tab: string | undefined;
  before: React.ReactNode;
}) {
  const { state } = useSidebar();
  const { data: settings } = useSettings();
  // The icon rail has no labels to fold under, so it always shows every icon.
  const expanded = open || state === "collapsed";

  return (
    <Collapsible open={expanded} onOpenChange={onOpenChange}>
      <SidebarGroup className="py-1">
        <CollapsibleTrigger asChild>
          <SidebarGroupLabel className="group/label h-control-sm cursor-pointer text-2xs font-medium tracking-wider text-sidebar-foreground/60 uppercase transition-colors hover:text-sidebar-foreground">
            {group.label}
            <ChevronRight
              className={cn(
                "ml-auto size-3 shrink-0 opacity-0 transition-[transform,opacity] duration-fast ease-out-strong group-hover/label:opacity-100 group-focus-visible/label:opacity-100",
                open && "rotate-90",
              )}
            />
          </SidebarGroupLabel>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <SidebarGroupContent>
            {before}
            <SidebarMenu>
              {group.items.map((item) => (
                <NavLink
                  key={item.id}
                  item={item}
                  pathname={pathname}
                  tab={tab}
                  badge={item.id === "llm" && settings && !settings.llm.enabled ? "off" : undefined}
                />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </CollapsibleContent>
      </SidebarGroup>
    </Collapsible>
  );
}

function CollectionsNav({
  collections,
  loading,
  pathname,
  onNewCollection,
  onImportCollections,
}: {
  collections: CollectionModel[];
  loading: boolean;
  pathname: string;
  onNewCollection: () => void;
  onImportCollections: () => void;
}) {
  const [systemOpen, setSystemOpen] = useState(false);

  const { data: settings } = useSettings();
  const teamsEnabled = Boolean(settings?.teams.enabled);
  const { userCollections, systemCollections } = useMemo(
    () => ({
      userCollections: collections.filter((c) => !c.system),
      systemCollections: collections.filter(
        (c) => c.system && (teamsEnabled || (c.name !== "_teams" && c.name !== "_team_members")),
      ),
    }),
    [collections, teamsEnabled],
  );

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <SidebarGroupAction title="New collection, export, or import" className="top-1">
            <Plus />
            <span className="sr-only">New collection, export, or import</span>
          </SidebarGroupAction>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={onNewCollection}>
            <Plus className="size-3.5" />
            New collection
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => downloadCollectionsExport(collections)}>
            <Download className="size-3.5" />
            Export collections
          </DropdownMenuItem>
          <DropdownMenuItem onClick={onImportCollections}>
            <Upload className="size-3.5" />
            Import collections
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <SidebarMenu>
        {loading ? (
          <>
            <SidebarMenuItem>
              <SidebarMenuSkeleton showIcon />
            </SidebarMenuItem>
            <SidebarMenuItem>
              <SidebarMenuSkeleton showIcon />
            </SidebarMenuItem>
            <SidebarMenuItem>
              <SidebarMenuSkeleton showIcon />
            </SidebarMenuItem>
          </>
        ) : userCollections.length === 0 ? (
          <li className="px-2 py-1.5 text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">
            No collections yet.{" "}
            <button
              type="button"
              onClick={onNewCollection}
              className="font-medium text-brand-text underline-offset-2 hover:underline"
            >
              Create one
            </button>
          </li>
        ) : (
          userCollections.map((collection) => (
            <CollectionItem key={collection.id} collection={collection} pathname={pathname} />
          ))
        )}
      </SidebarMenu>

      {systemCollections.length > 0 ? (
        <Collapsible open={systemOpen} onOpenChange={setSystemOpen} className="group/system group-data-[collapsible=icon]:hidden">
          <CollapsibleTrigger asChild>
            <button
              type="button"
              className="mt-1 flex h-control-xs w-full items-center gap-1 rounded-md px-2 text-xs text-muted-foreground outline-none transition-colors hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring"
            >
              <ChevronRight
                className={cn("size-3 shrink-0 transition-transform duration-fast ease-out-strong", systemOpen && "rotate-90")}
              />
              System collections
              <span className="ml-auto font-tabular text-2xs">{systemCollections.length}</span>
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <SidebarMenu>
              {systemCollections.map((collection) => (
                <CollectionItem key={collection.id} collection={collection} pathname={pathname} muted />
              ))}
            </SidebarMenu>
          </CollapsibleContent>
        </Collapsible>
      ) : null}
    </>
  );
}

function CollectionItem({
  collection,
  pathname,
  muted,
}: {
  collection: CollectionModel;
  pathname: string;
  muted?: boolean;
}) {
  const active = pathname === `/collections/${encodeURIComponent(collection.name)}`;
  const Icon = collection.type === "auth" ? ShieldUser : Database;

  return (
    <SidebarMenuItem>
      <SidebarMenuButton asChild isActive={active} tooltip={collection.name}>
        <Link to="/collections/$name" params={{ name: collection.name }}>
          <Icon className={muted ? "opacity-60" : undefined} />
          <span className={cn("truncate font-mono text-xs", muted && "text-sidebar-foreground/70")}>
            {collection.name}
          </span>
        </Link>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
