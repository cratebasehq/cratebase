import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Database, Plus, ShieldUser } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command";
import { Kbd } from "@/components/ui/kbd";
import { useDevMailInboxAvailable, useSettings } from "@/hooks/use-settings";
import { isSettingsTabVisible, SETTINGS_TAB_TARGETS } from "@/lib/settings-nav";

interface AppCommandPaletteProps {
  collections: CollectionModel[];
  onNewCollection: () => void;
  /** Optional controlled open state, so a visible search affordance in the
   * top bar can pop the palette without a synthetic keypress. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/** Global ⌘K palette: jump to any collection or trigger top-level actions
 * without leaving the keyboard. Every settings *tab* is still its own
 * jump target here (`SETTINGS_TAB_TARGETS`, `lib/settings-nav.ts`) even
 * though the sidebar only shows the 7 group pages — ⌘K → "webhooks"
 * still lands directly on Automation's Webhooks tab. */
export function AppCommandPalette({ collections, onNewCollection, open, onOpenChange }: AppCommandPaletteProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const navigate = useNavigate();
  const { data: settings } = useSettings();
  const { data: devMailInboxAvailable } = useDevMailInboxAvailable();
  const settingsTargets = SETTINGS_TAB_TARGETS.filter((tab) =>
    isSettingsTabVisible(tab.legacyPath, settings, devMailInboxAvailable),
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

  return (
    <CommandDialog
      open={isOpen}
      onOpenChange={setOpen}
      title="Command palette"
      description="Jump to a collection, open a settings page, or start an action."
    >
      <Command>
        <CommandInput placeholder="Jump to a collection or setting…" />
        <CommandList>
          <CommandEmpty>No matches.</CommandEmpty>

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
                {collection.name}
                <CommandShortcut>{collection.type}</CommandShortcut>
              </CommandItem>
            ))}
          </CommandGroup>

          <CommandGroup heading="Settings">
            {settingsTargets.map(({ to, value, label, groupLabel, groupIcon: Icon }) => (
              <CommandItem
                key={`${to}?tab=${value}`}
                value={`${label} ${groupLabel} settings`}
                onSelect={() =>
                  run(() => {
                    // `to`/`value` are picked at runtime from the flattened
                    // settings-tab registry, not a single literal route —
                    // TanStack's route tree can't narrow that statically.
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

          <CommandGroup heading="Actions">
            <CommandItem value="new collection add create" onSelect={() => run(onNewCollection)}>
              <Plus />
              New collection
              <CommandShortcut>
                <Kbd>Create</Kbd>
              </CommandShortcut>
            </CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    </CommandDialog>
  );
}
