import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, ChevronsUpDown, Loader2, X } from "lucide-react";
import type { RecordModel } from "pocketbase";
import { cb } from "@/lib/api";
import { cn } from "@/lib/utils";
import { userFields } from "@/lib/field-types";
import { useCollections } from "@/hooks/use-collections";
import { Badge } from "@/components/ui/badge";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";

/** How many candidates one page of the picker shows. The search runs on the
 * server, so this is a page size, not a ceiling on what can be picked. */
const PAGE_SIZE = 50;

interface Option {
  id: string;
  label: string;
  secondary: string;
}

/** The field a related record should be shown as. PocketBase marks one with
 * `presentable`; failing that, the first text field is the best guess, and
 * failing that the id is all there is. */
function displayFieldOf(fields: ReturnType<typeof userFields>): string | undefined {
  return fields.find((f) => f.presentable === true)?.name ?? fields.find((f) => f.type === "text")?.name;
}

function toOption(record: RecordModel, display: string | undefined, secondary: string | undefined): Option {
  return {
    id: record.id,
    label: display ? String(record[display] ?? record.id) : record.id,
    secondary: secondary ? String(record[secondary] ?? "") : "",
  };
}

/**
 * A searchable picker over a related collection.
 *
 * The old control was a Radix `Select` (or a checklist) over the *first 100
 * records* of the target — which silently cannot reach record 101, and is
 * unusable well before that. This searches server-side with the same `~`
 * filter the grid's quick search uses, and resolves the ids already on the
 * record separately so a selection stays labelled even when it isn't in the
 * current page of results.
 */
export function RelationPicker({
  collectionId,
  value,
  onChange,
  multiple,
  maxSelect,
  invalid,
  fieldName,
}: {
  collectionId: string | undefined;
  value: string[];
  onChange: (next: string[]) => void;
  multiple: boolean;
  maxSelect: number;
  invalid?: boolean;
  fieldName: string;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const { data: collections } = useCollections();

  const target = collections?.find((c) => c.id === collectionId);
  const targetFields = useMemo(() => (target ? userFields(target) : []), [target]);
  const display = displayFieldOf(targetFields);
  const secondary = targetFields.find((f) => f.type === "email" && f.name !== display)?.name;

  // Candidates for the dropdown, filtered by the server.
  const candidates = useQuery({
    queryKey: ["relation-candidates", collectionId, search],
    queryFn: async () => {
      const term = search.trim().replace(/"/g, '\\"');
      const filter = term && display ? `${display} ~ "${term}"` : undefined;
      const list = await cb.collection(collectionId!).getList<RecordModel>(1, PAGE_SIZE, {
        filter,
        skipTotal: true,
        requestKey: null,
      });
      return list.items.map((item) => toOption(item, display, secondary));
    },
    enabled: open && Boolean(collectionId),
    placeholderData: (previous) => previous,
  });

  // Labels for what is already selected — these may not be in the page of
  // candidates above, so they are fetched by id.
  const selected = useQuery({
    queryKey: ["relation-selected", collectionId, [...value].sort().join(",")],
    queryFn: async () => {
      if (value.length === 0) return [] as Option[];
      const filter = value.map((id) => `id = "${id}"`).join(" || ");
      const list = await cb.collection(collectionId!).getList<RecordModel>(1, Math.min(value.length, 200), {
        filter,
        skipTotal: true,
        requestKey: null,
      });
      return list.items.map((item) => toOption(item, display, secondary));
    },
    enabled: Boolean(collectionId) && value.length > 0,
  });

  const selectedById = new Map((selected.data ?? []).map((o) => [o.id, o]));
  const full = multiple && maxSelect > 0 && value.length >= maxSelect;

  function toggle(id: string) {
    if (!multiple) {
      onChange(value[0] === id ? [] : [id]);
      setOpen(false);
      return;
    }
    if (value.includes(id)) onChange(value.filter((v) => v !== id));
    else if (!full) onChange([...value, id]);
  }

  if (!collectionId) {
    return (
      <p className="rounded-md border border-dashed border-border px-2 py-1.5 text-xs text-muted-foreground">
        This relation has no target collection set.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <button
            type="button"
            role="combobox"
            aria-expanded={open}
            aria-invalid={invalid || undefined}
            aria-label={fieldName}
            className={cn(
              "flex h-control-md w-full items-center justify-between gap-2 rounded-lg border border-input bg-transparent px-2.5 text-sm transition-colors hover:bg-accent/40 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
              invalid && "border-destructive",
            )}
          >
            <span className={cn("min-w-0 truncate", value.length === 0 && "text-muted-foreground")}>
              {value.length === 0
                ? `Pick ${multiple ? "records" : "a record"} from ${target?.name ?? "…"}`
                : multiple
                  ? `${value.length} selected`
                  : (selectedById.get(value[0]!)?.label ?? value[0])}
            </span>
            <ChevronsUpDown className="size-3.5 shrink-0 text-muted-foreground" />
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-[--radix-popover-trigger-width] gap-0 p-0">
          <Command shouldFilter={false}>
            <CommandInput
              placeholder={display ? `Search ${target?.name} by ${display}…` : "Search by id…"}
              value={search}
              onValueChange={setSearch}
            />
            <CommandList>
              {candidates.isFetching && !candidates.data ? (
                <div className="flex items-center justify-center gap-2 py-6 text-sm text-muted-foreground">
                  <Loader2 className="size-3.5 animate-spin" />
                  Searching…
                </div>
              ) : (
                <>
                  <CommandEmpty>No matching records.</CommandEmpty>
                  <CommandGroup>
                    {(candidates.data ?? []).map((option) => {
                      const on = value.includes(option.id);
                      return (
                        <CommandItem
                          key={option.id}
                          value={option.id}
                          onSelect={() => toggle(option.id)}
                          disabled={!on && full}
                        >
                          <Check className={cn("size-3.5 shrink-0", on ? "opacity-100" : "opacity-0")} />
                          <span className="min-w-0 flex-1 truncate">{option.label}</span>
                          {option.secondary ? (
                            <span className="shrink-0 truncate text-2xs text-muted-foreground">
                              {option.secondary}
                            </span>
                          ) : null}
                          <span className="shrink-0 font-mono text-2xs text-muted-foreground/60">{option.id}</span>
                        </CommandItem>
                      );
                    })}
                  </CommandGroup>
                </>
              )}
            </CommandList>
          </Command>
          {full ? (
            <p className="border-t border-border px-3 py-2 text-2xs text-muted-foreground">
              {maxSelect} is the maximum for this field. Remove one to pick another.
            </p>
          ) : null}
        </PopoverContent>
      </Popover>

      {multiple && value.length > 0 ? (
        <div className="flex flex-wrap gap-1">
          {value.map((id) => (
            <Badge key={id} variant="outline" className="gap-1 pr-1 font-normal">
              <span className="max-w-40 truncate">{selectedById.get(id)?.label ?? id}</span>
              <button
                type="button"
                aria-label={`Remove ${selectedById.get(id)?.label ?? id}`}
                onClick={() => onChange(value.filter((v) => v !== id))}
                className="grid size-4 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              >
                <X className="size-3" />
              </button>
            </Badge>
          ))}
        </div>
      ) : null}
    </div>
  );
}
