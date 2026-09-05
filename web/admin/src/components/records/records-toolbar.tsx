import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { SortableContext, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Columns3, GripVertical, Rows2, Rows3, Settings2, Trash2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Separator } from "@/components/ui/separator";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { PAGE_SIZES } from "@/hooks/use-records";
import type { ColumnPrefs } from "@/hooks/use-column-prefs";
import type { Density } from "@/lib/grid";

function SortableColumnRow({
  id,
  label,
  visible,
  locked,
  onToggle,
}: {
  id: string;
  label: string;
  visible: boolean;
  locked: boolean;
  onToggle: (visible: boolean) => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });
  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn(
        "flex items-center gap-1.5 rounded-md pr-1.5 transition-colors",
        isDragging ? "bg-accent shadow-e2" : "hover:bg-accent/50",
      )}
    >
      <button
        type="button"
        aria-label={`Reorder ${label}`}
        className="grid size-control-xs shrink-0 cursor-grab touch-none place-items-center rounded text-muted-foreground/60 transition-colors hover:text-foreground active:cursor-grabbing"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-3.5" />
      </button>
      <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 py-1">
        <Checkbox
          checked={visible}
          disabled={locked}
          onCheckedChange={(next) => onToggle(next === true)}
          aria-label={`Show ${label}`}
        />
        <span className="truncate font-mono text-xs">{label}</span>
      </label>
      {locked ? <span className="shrink-0 text-2xs text-muted-foreground/60">always</span> : null}
    </div>
  );
}

export function ColumnsMenu({
  prefs,
  labels,
  locked,
}: {
  prefs: ColumnPrefs;
  labels: Record<string, string>;
  locked: string[];
}) {
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }), useSensor(KeyboardSensor));
  const hiddenCount = prefs.order.length - prefs.visible.length;

  function onDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    prefs.move(prefs.order.indexOf(String(active.id)), prefs.order.indexOf(String(over.id)));
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" className="h-control-sm gap-1.5">
          <Columns3 className="size-3.5" />
          Columns
          {hiddenCount > 0 ? (
            <span className="rounded bg-secondary px-1 font-tabular text-2xs text-muted-foreground">
              {prefs.visible.length}/{prefs.order.length}
            </span>
          ) : null}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-64 gap-0 p-0">
        <div className="flex items-center justify-between px-3 py-2">
          <span className="text-xs font-medium">Columns</span>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={prefs.showAll}
              className="rounded px-1.5 py-0.5 text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            >
              Show all
            </button>
            {prefs.customized ? (
              <button
                type="button"
                onClick={prefs.reset}
                className="rounded px-1.5 py-0.5 text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              >
                Reset
              </button>
            ) : null}
          </div>
        </div>
        <Separator />
        <div className="max-h-80 overflow-y-auto overscroll-contain">
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
            <SortableContext items={prefs.order} strategy={verticalListSortingStrategy}>
              <div className="flex flex-col p-1.5">
                {prefs.order.map((id) => (
                  <SortableColumnRow
                    key={id}
                    id={id}
                    label={labels[id] ?? id}
                    visible={!prefs.isHidden(id)}
                    locked={locked.includes(id)}
                    onToggle={(visible) => prefs.toggle(id, visible)}
                  />
                ))}
              </div>
            </SortableContext>
          </DndContext>
        </div>
        <Separator />
        <p className="px-3 py-2 text-2xs leading-snug text-muted-foreground">
          Layout is remembered per collection, in this browser.
        </p>
      </PopoverContent>
    </Popover>
  );
}

export function ViewMenu({
  perPage,
  onPerPageChange,
  density,
  onDensityChange,
  countTotal,
  onCountTotalChange,
}: {
  perPage: number;
  onPerPageChange: (next: number) => void;
  density: Density;
  onDensityChange: (next: Density) => void;
  countTotal: boolean;
  onCountTotalChange: (next: boolean) => void;
}) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" className="h-control-sm gap-1.5">
          <Settings2 className="size-3.5" />
          View
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 gap-3 p-3">
        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium">Rows per page</span>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            spacing={0}
            aria-label="Rows per page"
            value={String(perPage)}
            onValueChange={(next) => next && onPerPageChange(Number(next))}
            className="w-full"
          >
            {PAGE_SIZES.map((size) => (
              <ToggleGroupItem key={size} value={String(size)} className="flex-1 font-tabular">
                {size}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <p className="text-2xs leading-snug text-muted-foreground">
            Rows are virtualized, so a large page costs about the same to render as a small one. The server caps a
            page at 1000.
          </p>
        </div>

        <Separator />

        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium">Row height</span>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            spacing={0}
            aria-label="Row height"
            value={density}
            onValueChange={(next) => next && onDensityChange(next as Density)}
            className="w-full"
          >
            <ToggleGroupItem value="comfortable" className="flex-1 gap-1.5">
              <Rows3 className="size-3.5" />
              Comfortable
            </ToggleGroupItem>
            <ToggleGroupItem value="compact" className="flex-1 gap-1.5">
              <Rows2 className="size-3.5" />
              Compact
            </ToggleGroupItem>
          </ToggleGroup>
        </div>

        <Separator />

        <label className="flex cursor-pointer items-start gap-2">
          <Checkbox
            checked={countTotal}
            onCheckedChange={(next) => onCountTotalChange(next === true)}
            className="mt-0.5"
          />
          <span className="flex flex-col gap-0.5">
            <span className="text-xs font-medium">Count total rows</span>
            <span className="text-2xs leading-snug text-muted-foreground">
              Off sends <code className="font-mono">skipTotal</code>, which drops the <code className="font-mono">COUNT(*)</code>{" "}
              from every query — noticeably faster on large tables, at the cost of the total and the last-page jump.
            </span>
          </span>
        </label>
      </PopoverContent>
    </Popover>
  );
}

export function SelectionBar({
  count,
  onClear,
  onDelete,
  batchEnabled,
  busy,
}: {
  count: number;
  onClear: () => void;
  onDelete: () => void;
  batchEnabled: boolean;
  busy: boolean;
}) {
  if (count === 0) return null;
  return (
    <div className="flex h-control-lg shrink-0 items-center gap-3 border-b border-border bg-primary/[0.06] px-page">
      <span className="font-tabular text-sm font-medium">
        {count.toLocaleString()} selected
      </span>
      <Separator orientation="vertical" className="h-4" />
      {batchEnabled ? (
        <Button variant="ghost" size="sm" className="h-control-sm gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive" onClick={onDelete} disabled={busy}>
          <Trash2 className="size-3.5" />
          Delete
        </Button>
      ) : (
        <Tooltip>
          <TooltipTrigger asChild>
            <span>
              <Button variant="ghost" size="sm" className="h-control-sm gap-1.5" disabled>
                <Trash2 className="size-3.5" />
                Delete
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>
            Bulk delete uses <code className="font-mono">POST /api/batch</code>, which is disabled in settings.
          </TooltipContent>
        </Tooltip>
      )}
      <div className="flex-1" />
      <Button variant="ghost" size="sm" className="h-control-sm gap-1.5" onClick={onClear}>
        <X className="size-3.5" />
        Clear
      </Button>
    </div>
  );
}
