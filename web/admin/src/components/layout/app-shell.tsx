import { useMemo, useState } from "react";
import type { CollectionModel } from "cratebase";
import { Link, Outlet, useNavigate } from "@tanstack/react-router";
import { Boxes, ChevronRight, Database, LogOut, Moon, Plus, Settings, ShieldUser, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import { toast } from "sonner";
import { useCollections } from "@/hooks/use-collections";
import { cb } from "@/lib/api";
import { cn } from "@/lib/utils";
import { AppCommandPalette } from "@/components/app-command-palette";
import { NewCollectionDialog } from "@/components/collections/new-collection-dialog";

export function AppShell() {
  const { data: collections } = useCollections();
  const { resolvedTheme, setTheme } = useTheme();
  const navigate = useNavigate();
  const [newCollectionOpen, setNewCollectionOpen] = useState(false);
  const [systemOpen, setSystemOpen] = useState(false);

  // Collections named with a leading `_` are Cratebase-managed system
  // collections (auth admins, cron jobs, feature flags, …); the sidebar
  // groups them behind a collapsed section so it stays focused on the
  // collections a developer actually created, matching PocketBase's convention.
  const { userCollections, systemCollections } = useMemo(() => {
    const all = collections ?? [];
    return {
      userCollections: all.filter((c) => !c.name.startsWith("_")),
      systemCollections: all.filter((c) => c.name.startsWith("_")),
    };
  }, [collections]);

  function logout() {
    cb.authStore.clear();
    toast.message("Signed out");
    void navigate({ to: "/login" });
  }

  return (
    <div className="flex h-svh overflow-hidden bg-background text-foreground">
      <aside className="flex w-64 shrink-0 flex-col border-r border-sidebar-border bg-sidebar">
        <div className="flex items-center gap-2 px-4 py-4">
          <img src="/favicon.svg" alt="" className="size-6" />
          <span className="text-sm font-semibold tracking-tight">Cratebase</span>
        </div>

        <div className="flex items-center justify-between px-4 pb-2">
          <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Collections</span>
          <button
            type="button"
            onClick={() => setNewCollectionOpen(true)}
            className="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
            aria-label="New collection"
          >
            <Plus className="size-3.5" />
          </button>
        </div>

        <nav className="flex-1 overflow-y-auto px-2">
          {userCollections.map((collection) => (
            <CollectionLink key={collection.id} collection={collection} />
          ))}
          {collections?.length === 0 ? (
            <p className="px-2.5 py-4 text-[12px] text-muted-foreground">
              No collections yet. Create one to start storing data.
            </p>
          ) : null}

          {systemCollections.length > 0 ? (
            <div className="mt-2 border-t border-sidebar-border pt-2">
              <button
                type="button"
                onClick={() => setSystemOpen((open) => !open)}
                className="flex w-full items-center gap-1 rounded-lg px-2.5 py-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground transition-colors hover:text-sidebar-foreground"
                aria-expanded={systemOpen}
              >
                <ChevronRight className={cn("size-3 shrink-0 transition-transform", systemOpen && "rotate-90")} />
                System
              </button>
              {systemOpen
                ? systemCollections.map((collection) => (
                    <CollectionLink key={collection.id} collection={collection} muted />
                  ))
                : null}
            </div>
          ) : null}
        </nav>

        <div className="flex items-center justify-between border-t border-sidebar-border px-3 py-2">
          <button
            type="button"
            onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}
            className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
            aria-label="Toggle theme"
          >
            {resolvedTheme === "dark" ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
          </button>
          <Link
            to="/settings/logs"
            className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
            activeProps={{ className: "!bg-sidebar-accent !text-sidebar-accent-foreground" }}
          >
            <Settings className="size-3.5" />
            Settings
          </Link>
        </div>
        <div className="flex items-center justify-between border-t border-sidebar-border px-3 py-3">
          <button
            type="button"
            onClick={logout}
            className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
          >
            <LogOut className="size-3.5" />
            Sign out
          </button>
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        <Outlet />
      </main>

      <AppCommandPalette
        collections={collections ?? []}
        onNewCollection={() => setNewCollectionOpen(true)}
      />
      <NewCollectionDialog open={newCollectionOpen} onOpenChange={setNewCollectionOpen} />
    </div>
  );
}

function CollectionLink({ collection, muted }: { collection: CollectionModel; muted?: boolean }) {
  return (
    <Link
      to="/collections/$name"
      params={{ name: collection.name }}
      className={cn(
        "group flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-[13px] text-sidebar-foreground/80 transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
        muted && "text-sidebar-foreground/60"
      )}
      activeProps={{ className: "!bg-sidebar-accent !text-sidebar-accent-foreground font-medium" }}
    >
      {collection.type === "auth" ? (
        <ShieldUser className="size-3.5 shrink-0 opacity-70" />
      ) : (
        <Database className="size-3.5 shrink-0 opacity-70" />
      )}
      <span className="truncate">{collection.name}</span>
    </Link>
  );
}

export function EmptyDashboard() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 text-center">
      <Boxes className="size-10 text-muted-foreground" />
      <div>
        <p className="text-sm font-medium">Select a collection</p>
        <p className="text-sm text-muted-foreground">or create one from the sidebar to get started.</p>
      </div>
    </div>
  );
}
