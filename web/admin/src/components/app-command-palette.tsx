import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Database, Plus, ShieldUser } from "lucide-react";
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
import { SETTINGS_GROUPS } from "@/lib/settings-nav";

interface AppCommandPaletteProps {
  collections: CollectionModel[];
  onNewCollection: () => void;
  /** Optional controlled open state, so a visible search affordance in the
   * top bar can pop the palette without a synthetic keypress. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/** Every settings item, tagged with its group label for the palette's
 * `CommandShortcut` — the single source of truth is `SETTINGS_GROUPS`
 * (`lib/settings-nav.ts`), also consumed by the sidebar and breadcrumbs. */
const SETTINGS_TARGETS = SETTINGS_GROUPS.flatMap((group) =>
  group.items.map((item) => ({ ...item, group: group.label })),
);

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
            {SETTINGS_TARGETS.map(({ to, label, group, icon: Icon }) => (
              <CommandItem
                key={to}
                value={`${label} ${group} settings`}
                onSelect={() =>
                  run(() => {
                    void navigate({ to });
                  })
                }
              >
                <Icon />
                {label}
                <CommandShortcut>{group}</CommandShortcut>
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
