import { createContext, use, useMemo, useState } from "react";
import { Outlet } from "@tanstack/react-router";
import { AppCommandPalette } from "@/components/app-command-palette";
import { ImportCollectionsDialog } from "@/components/collections/import-collections-dialog";
import { NewCollectionDialog } from "@/components/collections/new-collection-dialog";
import { AppSidebar } from "@/components/layout/app-sidebar";
import { AppTopbar } from "@/components/layout/app-topbar";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useCollections } from "@/hooks/use-collections";

interface ShellActions {
  openNewCollection: () => void;
  openSearch: () => void;
}

const ShellActionsContext = createContext<ShellActions>({
  openNewCollection: () => {},
  openSearch: () => {},
});

/** The two global actions the shell owns, reachable from any screen inside
 * it — so an empty state can offer a real button instead of describing
 * where the button lives. */
export function useShellActions(): ShellActions {
  return use(ShellActionsContext);
}

/**
 * The application frame: a sidebar that collapses to an icon rail on desktop
 * and becomes a sheet on mobile, a top bar carrying location, search and
 * connection state, and the routed screen below it.
 *
 * `--sidebar-width` is pinned to the `sidebar` density token so the rail
 * agrees with the rest of the spacing scale rather than shadcn's default.
 */
export function AppShell() {
  const { data: collections, isPending } = useCollections();
  const [newCollectionOpen, setNewCollectionOpen] = useState(false);
  const [importCollectionsOpen, setImportCollectionsOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);

  const actions = useMemo<ShellActions>(
    () => ({
      openNewCollection: () => setNewCollectionOpen(true),
      openSearch: () => setSearchOpen(true),
    }),
    [],
  );
  return (
    <ShellActionsContext value={actions}>
      <TooltipProvider delayDuration={300}>
        <SidebarProvider
          className="h-svh overflow-hidden"
          style={
            {
              "--sidebar-width": "var(--spacing-sidebar)",
              "--sidebar-width-icon": "var(--spacing-sidebar-icon)",
            } as React.CSSProperties
          }
        >
          <AppSidebar
            collections={collections ?? []}
            loading={isPending}
            onNewCollection={actions.openNewCollection}
            onImportCollections={() => setImportCollectionsOpen(true)}
          />

          <SidebarInset className="flex min-w-0 flex-col overflow-hidden">
            <AppTopbar onOpenSearch={actions.openSearch} />
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
              <Outlet />
            </div>
          </SidebarInset>

          <AppCommandPalette
            collections={collections ?? []}
            onNewCollection={actions.openNewCollection}
            open={searchOpen}
            onOpenChange={setSearchOpen}
          />
          <NewCollectionDialog open={newCollectionOpen} onOpenChange={setNewCollectionOpen} />
          <ImportCollectionsDialog open={importCollectionsOpen} onOpenChange={setImportCollectionsOpen} />
        </SidebarProvider>
      </TooltipProvider>
    </ShellActionsContext>
  );
}
