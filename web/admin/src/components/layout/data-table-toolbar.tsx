import type { ReactNode } from "react";
import { Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { cn } from "@/lib/utils";

/**
 * The strip above a table or list: search on the left, filters next to it,
 * actions pinned right. When rows are selected, `selection` replaces the
 * filters with a count and the bulk actions, so there is only ever one
 * toolbar and it never changes height.
 *
 *   <DataTableToolbar
 *     search={{ value, onChange, placeholder: "Search webhooks…" }}
 *     filters={<Select …/>}
 *     actions={<Button size="sm">New webhook</Button>}
 *     selection={selected.length ? { count: selected.length, onClear, actions: <Button variant="destructive" size="sm">Delete</Button> } : undefined}
 *   />
 */
export function DataTableToolbar({
  search,
  filters,
  actions,
  selection,
  className,
}: {
  search?: { value: string; onChange: (value: string) => void; placeholder?: string; label?: string };
  filters?: ReactNode;
  actions?: ReactNode;
  selection?: { count: number; onClear: () => void; actions: ReactNode };
  className?: string;
}) {
  return (
    <div className={cn("flex min-h-control-md flex-wrap items-center gap-2", className)}>
      {selection ? (
        <div className="flex flex-1 flex-wrap items-center gap-2">
          <span className="font-tabular text-sm font-medium">{selection.count} selected</span>
          {selection.actions}
          <Button variant="ghost" size="sm" onClick={selection.onClear}>
            Clear
          </Button>
        </div>
      ) : (
        <>
          {search ? (
            <InputGroup className="h-control-sm w-full sm:w-64">
              <InputGroupAddon>
                <Search />
              </InputGroupAddon>
              <InputGroupInput
                type="search"
                aria-label={search.label ?? search.placeholder ?? "Search"}
                placeholder={search.placeholder ?? "Search…"}
                value={search.value}
                onChange={(event) => search.onChange(event.target.value)}
                autoComplete="off"
                spellCheck={false}
              />
              {search.value ? (
                <InputGroupAddon align="inline-end">
                  <InputGroupButton aria-label="Clear search" onClick={() => search.onChange("")}>
                    <X />
                  </InputGroupButton>
                </InputGroupAddon>
              ) : null}
            </InputGroup>
          ) : null}
          {filters ? <div className="flex flex-wrap items-center gap-2">{filters}</div> : null}
        </>
      )}
      {actions ? <div className="ml-auto flex shrink-0 items-center gap-2">{actions}</div> : null}
    </div>
  );
}
