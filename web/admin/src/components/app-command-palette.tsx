import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useTheme } from "next-themes";
import { Database, LogOut, Monitor, Moon, PanelLeft, Plus, ShieldUser, Sun } from "lucide-react";
import { toast } from "sonner";
import type { CollectionModel } from "@cratebase/client";
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command";
import { Kbd } from "@/components/ui/kbd";
import { useSidebar } from "@/components/ui/sidebar";
import { useDevMailInboxAvailable, useSettings } from "@/hooks/use-settings";
import { signOut } from "@/lib/api";
import { ALL_NAV_ITEMS, NAV_GROUPS, OVERVIEW, navLinkProps, type NavItem } from "@/lib/nav";
import { isSettingsTabVisible, SETTINGS_GROUPS, SETTINGS_TAB_TARGETS } from "@/lib/settings-nav";

interface AppCommandPaletteProps {
  collections: CollectionModel[];
  onNewCollection: () => void;
  /** Optional controlled open state, so a visible search affordance in the
   * top bar can pop the palette without a synthetic keypress. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

function Chord({ item }: { item: NavItem }) {
  return item.chord ? (
    <CommandShortcut className="flex gap-1">
      <Kbd>G</Kbd>
      <Kbd>{item.chord.toUpperCase()}</Kbd>
    </CommandShortcut>
  ) : null;
}

/** Global ⌘K palette. Reaches every screen the sidebar does (same `NAV_GROUPS`
 * registry), every settings *tab* the sidebar folds away, every collection,
 * and the handful of global actions. Opening is instant by design — it is a
 * keyboard action people repeat all day. */
export function AppCommandPalette({ collections, onNewCollection, open, onOpenChange }: AppCommandPaletteProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { setTheme } = useTheme();
  const { toggleSidebar } = useSidebar();
  const { data: settings } = useSettings();
  const { data: devMailInboxAvailable } = useDevMailInboxAvailable();

  // Settings tabs the sidebar has no row for (Branding, Extensions, RPC…).
  const navKeys = new Set(
    ALL_NAV_ITEMS.map((item) => `${item.to}|${item.tab ?? SETTINGS_GROUPS.find((g) => g.to === item.to)?.tabs[0]?.value}`),
  );
  const otherSettings = SETTINGS_TAB_TARGETS.filter(
    (tab) => isSettingsTabVisible(tab.legacyPath, settings, devMailInboxAvailable) && !navKeys.has(`${tab.to}|${tab.value}`),
  );

  const isOpen = open ?? internalOpen;

  const setOpen = useCallback(
    (next: boolean) => {
      setInternalOpen(next);
      onOpenChange?.(next);
    },
    [onOpenChange],
  );

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setOpen(!isOpen);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isOpen, setOpen]);

  function run(action: () => void) {
    setOpen(false);
    action();
  }

  function goTo(item: NavItem) {
    run(() => void navigate(navLinkProps(item) as never));
  }

  return (
    <CommandDialog
      open={isOpen}
      onOpenChange={setOpen}
      title="Command palette"
      description="Jump to a collection or settings page, or run an action."
      className="data-[state=closed]:animate-none data-[state=open]:animate-none sm:max-w-xl"
    >
      <Command>
        <CommandInput placeholder="Search collections, settings, actions…" />
        <CommandList className="max-h-[min(24rem,60svh)]">
          <CommandEmpty>Nothing matches. Try a collection name or a setting.</CommandEmpty>

          <CommandGroup heading="Actions">
            <CommandItem value="new collection add create" onSelect={() => run(onNewCollection)}>
              <Plus />
              New collection
            </CommandItem>
            <CommandItem value="toggle sidebar collapse expand" onSelect={() => run(toggleSidebar)}>
              <PanelLeft />
              Toggle sidebar
              <CommandShortcut className="flex gap-1">
                <Kbd>⌘B</Kbd>
              </CommandShortcut>
            </CommandItem>
            <CommandItem value="theme light appearance" onSelect={() => run(() => setTheme("light"))}>
              <Sun />
              Light theme
            </CommandItem>
            <CommandItem value="theme dark appearance" onSelect={() => run(() => setTheme("dark"))}>
              <Moon />
              Dark theme
            </CommandItem>
            <CommandItem value="theme system appearance match" onSelect={() => run(() => setTheme("system"))}>
              <Monitor />
              Match system theme
            </CommandItem>
            <CommandItem
              value="sign out log out"
              onSelect={() =>
                run(() => {
                  signOut();
                  queryClient.clear();
                  toast.success("Signed out");
                  void navigate({ to: "/login" });
                })
              }
            >
              <LogOut />
              Sign out
            </CommandItem>
          </CommandGroup>

          <CommandSeparator />

          <CommandGroup heading="Collections">
            {collections.map((collection) => (
              <CommandItem
                key={collection.id}
                value={`${collection.name} ${collection.type} collection records`}
                onSelect={() =>
                  run(() => {
                    void navigate({ to: "/collections/$name", params: { name: collection.name } });
                  })
                }
              >
                {collection.type === "auth" ? <ShieldUser /> : <Database />}
                <span className="font-mono text-xs">{collection.name}</span>
                <CommandShortcut>{collection.system ? "system" : collection.type}</CommandShortcut>
              </CommandItem>
            ))}
          </CommandGroup>

          <CommandGroup heading="Go to">
            <CommandItem value={`${OVERVIEW.label} home dashboard`} onSelect={() => goTo(OVERVIEW)}>
              <OVERVIEW.icon />
              {OVERVIEW.label}
              <Chord item={OVERVIEW} />
            </CommandItem>
          </CommandGroup>

          {NAV_GROUPS.map((group) => (
            <CommandGroup key={group.id} heading={group.label}>
              {group.items.map((item) => (
                <CommandItem key={item.id} value={`${item.label} ${group.label} ${item.keywords ?? ""}`} onSelect={() => goTo(item)}>
                  <item.icon />
                  {item.label}
                  <Chord item={item} />
                </CommandItem>
              ))}
            </CommandGroup>
          ))}

          {otherSettings.length > 0 ? (
            <CommandGroup heading="More settings">
              {otherSettings.map(({ to, value, label, groupLabel, groupIcon: Icon }) => (
                <CommandItem
                  key={`${to}?tab=${value}`}
                  value={`${label} ${groupLabel} settings`}
                  onSelect={() =>
                    run(() => {
                      // `to`/`value` come from the flattened settings-tab
                      // registry, not one literal route, so TanStack's route
                      // tree cannot narrow them statically.
                      void navigate({ to, search: { tab: value } } as never);
                    })
                  }
                >
                  <Icon />
                  {label}
                  <CommandShortcut>{groupLabel}</CommandShortcut>
                </CommandItem>
              ))}
            </CommandGroup>
          ) : null}
        </CommandList>

        <div className="flex items-center gap-3 border-t border-border px-3 py-2 text-2xs text-muted-foreground">
          <span className="flex items-center gap-1">
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd> navigate
          </span>
          <span className="flex items-center gap-1">
            <Kbd>↵</Kbd> open
          </span>
          <span className="ml-auto flex items-center gap-1">
            <Kbd>esc</Kbd> close
          </span>
        </div>
      </Command>
    </CommandDialog>
  );
}
