import { Search } from "lucide-react";
import { Breadcrumbs } from "@/components/layout/breadcrumbs";
import { ConnectionStatus } from "@/components/layout/connection-status";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Separator } from "@/components/ui/separator";
import { SidebarTrigger } from "@/components/ui/sidebar";

/**
 * The top bar the old shell did without entirely: where you are, how to
 * search, and whether the server is still there.
 */
export function AppTopbar({ onOpenSearch }: { onOpenSearch: () => void }) {
  return (
    <header className="sticky top-0 z-sticky flex h-topbar shrink-0 items-center gap-2 border-b border-border bg-background/80 pr-3 pl-2 backdrop-blur-sm">
      <SidebarTrigger className="size-7" />
      <Separator orientation="vertical" className="mr-1 data-[orientation=vertical]:h-4" />
      <Breadcrumbs />

      <div className="ml-auto flex shrink-0 items-center gap-2">
        {/* Reads as a search field, behaves as the command palette trigger —
            the palette already worked, it just had no visible affordance.
            The label truncates and the shortcut is pinned: the two used to
            fight over a 224px button, and the ⌘K lost. */}
        <Button
          variant="outline"
          size="sm"
          onClick={onOpenSearch}
          className="hidden h-control-sm w-64 justify-start gap-2 px-2 font-normal text-muted-foreground md:flex"
        >
          <Search className="size-3.5 shrink-0" />
          <span className="min-w-0 truncate">Search collections</span>
          <Kbd className="ml-auto shrink-0">⌘K</Kbd>
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={onOpenSearch}
          aria-label="Search"
          className="md:hidden"
        >
          <Search />
        </Button>

        <ConnectionStatus />
      </div>
    </header>
  );
}
