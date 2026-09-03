import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import type { CollectionModel } from "cratebase";
import { CommandPalette, type CommandItem } from "@/components/interior/command-palette";

interface AppCommandPaletteProps {
  collections: CollectionModel[];
  onNewCollection: () => void;
}

/** Global ⌘K palette: jump to any collection or trigger top-level actions
 * without leaving the keyboard. */
export function AppCommandPalette({ collections, onNewCollection }: AppCommandPaletteProps) {
  const [open, setOpen] = useState(false);
  const navigate = useNavigate();

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setOpen((prev) => !prev);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const items: CommandItem[] = [
    { id: "__new-collection", label: "New collection", hint: "Create", keywords: "add create collection" },
    ...collections.map((collection) => ({
      id: collection.id,
      label: collection.name,
      hint: collection.type,
      keywords: `${collection.name} ${collection.type} collection records`,
    })),
  ];

  function handleSelect(item: CommandItem) {
    setOpen(false);
    if (item.id === "__new-collection") {
      onNewCollection();
      return;
    }
    const collection = collections.find((c) => c.id === item.id);
    if (collection) void navigate({ to: "/collections/$name", params: { name: collection.name } });
  }

  return (
    <CommandPalette
      open={open}
      items={items}
      onSelect={handleSelect}
      onDismiss={() => setOpen(false)}
      placeholder="Jump to a collection…"
      label="Command palette"
      autoFocus
    />
  );
}
