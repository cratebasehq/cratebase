import { useMemo, useState } from "react";
import type { CollectionModel } from "@cratebase/client";
import { Link, useRouterState } from "@tanstack/react-router";
import {
  ChevronRight,
  Database,
  Download,
  LayoutGrid,
  Plus,
  Settings,
  ShieldUser,
  Upload,
} from "lucide-react";
import { CratebaseMark } from "@/components/brand/cratebase-mark";
import { AccountMenu } from "@/components/layout/account-menu";
import { SettingsNav } from "@/components/settings/settings-nav";
import { useSettings } from "@/hooks/use-settings";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
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

/** PocketBase's own dashboard names a schema export `pb_schema.json` and
 * ships the collection array as-is (no envelope) — matching that means an
 * export from here re-imports into a real PocketBase instance unchanged,
 * and vice versa. */
function downloadCollectionsExport(collections: CollectionModel[]) {
  const blob = new Blob([JSON.stringify(collections, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "pb_schema.json";
  anchor.click();
  URL.revokeObjectURL(url);
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
  const pathname = useRouterState({ select: (state) => state.location.pathname });

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
                  <Link to="/settings">
                    <Settings />
                    <span>Settings</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        {pathname.startsWith("/settings") ? (
          <SettingsNav pathname={pathname} />
        ) : (
          <CollectionsNav
            collections={collections}
            loading={loading}
            pathname={pathname}
            onNewCollection={onNewCollection}
            onImportCollections={onImportCollections}
          />
        )}

      </SidebarContent>

      <SidebarSeparator className="mx-0" />
      <SidebarFooter>
        <AccountMenu />
      </SidebarFooter>
    </Sidebar>
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
      <SidebarGroup>
        <SidebarGroupLabel>Collections</SidebarGroupLabel>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <SidebarGroupAction title="New collection, export, or import">
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
                <CollectionItem key={collection.id} collection={collection} pathname={pathname} />
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
                    <CollectionItem key={collection.id} collection={collection} pathname={pathname} muted />
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </CollapsibleContent>
          </SidebarGroup>
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
          <span className={cn("truncate", muted && "text-sidebar-foreground/70")}>
            {collection.name}
          </span>
        </Link>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
