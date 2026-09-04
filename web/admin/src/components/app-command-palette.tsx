import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Archive, Clock, Database, ListTree, Plus, ShieldUser } from "lucide-react";
import type { CollectionModel } from "pocketbase";
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

interface AppCommandPaletteProps {
  collections: CollectionModel[];
  onNewCollection: () => void;
  /** Optional controlled open state, so a visible search affordance in the
   * top bar can pop the palette without a synthetic keypress. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

const SETTINGS_TARGETS = [
  { to: "/settings/logs", label: "Request logs", icon: ListTree },
  { to: "/settings/backups", label: "Backups", icon: Archive },
  { to: "/settings/cron", label: "Cron jobs", icon: Clock },
] as const;

/** Global ⌘K palette: jump to any collection or trigger top-level actions
 * without leaving the keyboard. */
export function AppCommandPalette({ collections, onNewCollection, open, onOpenChange }: AppCommandPaletteProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const navigate = useNavigate();

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
        <CommandInput placeholder="Jump to a collection…" />
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
            {SETTINGS_TARGETS.map(({ to, label, icon: Icon }) => (
              <CommandItem
                key={to}
                value={`${label} settings`}
                onSelect={() =>
                  run(() => {
                    void navigate({ to });
                  })
                }
              >
                <Icon />
                {label}
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
