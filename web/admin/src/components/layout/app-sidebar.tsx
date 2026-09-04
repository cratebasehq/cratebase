import { useMemo, useState } from "react";
import type { CollectionModel } from "cratebase";
import { Link, useRouterState } from "@tanstack/react-router";
import { ChevronRight, Database, LayoutGrid, Plus, Settings, ShieldUser } from "lucide-react";
import { CratebaseMark } from "@/components/brand/cratebase-mark";
import { AccountMenu } from "@/components/layout/account-menu";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
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
} from "@/components/ui/sidebar";
import { cn } from "@/lib/utils";

export function AppSidebar({
  collections,
  loading,
  onNewCollection,
}: {
  collections: CollectionModel[];
  loading: boolean;
  onNewCollection: () => void;
}) {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const [systemOpen, setSystemOpen] = useState(false);

  // Collections named with a leading `_` are Cratebase-managed system
  // collections (superusers, auth origins, MFA, OTP, external auths); the
  // sidebar keeps them behind a collapsed group so it stays focused on the
  // collections a developer actually created, matching PocketBase's
  // convention.
  const { userCollections, systemCollections } = useMemo(
    () => ({
      userCollections: collections.filter((c) => !c.name.startsWith("_")),
      systemCollections: collections.filter((c) => c.name.startsWith("_")),
    }),
    [collections],
  );

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader className="h-topbar justify-center border-b border-sidebar-border px-2">
        <Link
          to="/"
          className="flex items-center gap-2 rounded-md px-1 py-1 outline-none focus-visible:ring-3 focus-visible:ring-sidebar-ring/50"
        >
          <CratebaseMark className="size-5 shrink-0" />
          <span className="text-sm font-medium tracking-tight group-data-[collapsible=icon]:hidden">
            Cratebase
          </span>
        </Link>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton asChild isActive={pathname === "/"} tooltip="Overview">
                  <Link to="/">
                    <LayoutGrid />
                    <span>Overview</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  asChild
                  isActive={pathname.startsWith("/settings")}
                  tooltip="Settings"
                >
                  <Link to="/settings/logs">
                    <Settings />
                    <span>Settings</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <SidebarGroup>
          <SidebarGroupLabel>Collections</SidebarGroupLabel>
          <SidebarGroupAction onClick={onNewCollection} title="New collection">
            <Plus />
            <span className="sr-only">New collection</span>
          </SidebarGroupAction>
          <SidebarGroupContent>
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
                <p className="px-2 py-1.5 text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">
                  Nothing here yet. Create a collection to start storing data.
                </p>
              ) : (
                userCollections.map((collection) => (
                  <CollectionItem
                    key={collection.id}
                    collection={collection}
                    pathname={pathname}
                  />
                ))
              )}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        {systemCollections.length > 0 ? (
          <Collapsible open={systemOpen} onOpenChange={setSystemOpen} className="group/system">
            <SidebarGroup>
              <CollapsibleTrigger asChild>
                <SidebarGroupLabel className="cursor-pointer hover:text-sidebar-foreground">
                  <ChevronRight
                    className={cn(
                      "mr-1 size-3 shrink-0 transition-transform duration-fast ease-out-strong",
                      systemOpen && "rotate-90",
                    )}
                  />
                  System
                  <span className="ml-auto font-tabular text-2xs text-muted-foreground">
                    {systemCollections.length}
                  </span>
                </SidebarGroupLabel>
              </CollapsibleTrigger>
              <CollapsibleContent>
                <SidebarGroupContent>
                  <SidebarMenu>
                    {systemCollections.map((collection) => (
                      <CollectionItem
                        key={collection.id}
                        collection={collection}
                        pathname={pathname}
                        muted
                      />
                    ))}
                  </SidebarMenu>
                </SidebarGroupContent>
              </CollapsibleContent>
            </SidebarGroup>
          </Collapsible>
        ) : null}
      </SidebarContent>

      <SidebarSeparator className="mx-0" />
      <SidebarFooter>
        <AccountMenu />
      </SidebarFooter>
    </Sidebar>
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
          <span className={cn("truncate", muted && "text-sidebar-foreground/70")}>
            {collection.name}
          </span>
        </Link>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
