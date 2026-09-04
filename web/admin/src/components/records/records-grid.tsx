import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, ChevronsUpDown, Rows3, TriangleAlert } from "lucide-react";
import { cn } from "@/lib/utils";
import { ROW_HEIGHT, type Density } from "@/lib/grid";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";


/** Widths of the two fixed gutters, in px, for the same reason. */
const SELECT_WIDTH = 34;
const ACTIONS_WIDTH = 38;

/** How far past the viewport the virtualizer keeps rows mounted. Enough
 * that a flung scroll never shows a hole, few enough that a 500-row page
 * mounts ~40 rows rather than 500. */
const OVERSCAN = 14;

export interface GridCellState {
  /** True when this exact cell is the one being edited. */
  editing: boolean;
  /** True when this cell holds the keyboard cursor. */
  active: boolean;
  beginEdit: () => void;
  endEdit: () => void;
}

export interface GridColumn<T> {
  id: string;
  header: string;
  /** Rendered width in px. Columns are fixed-width so the header, the
   * virtualized rows and the horizontal scrollbar all agree. */
  width: number;
  /** The `sort` expression the server takes for this column — `title`,
   * `author.name`. Omit to make the column unsortable. */
  sortKey?: string;
  /** True when Enter on the cell should start an inline edit. */
  editable?: boolean;
  cell: (row: T, state: GridCellState) => ReactNode;
}

export type GridStatus = "loading" | "error" | "ready";

interface RecordsGridProps<T> {
  label: string;
  rows: T[];
  columns: GridColumn<T>[];
  getRowId: (row: T) => string;
  /** The raw server `sort` param (`-created`, `author.name`, `@random`). */
  sort: string;
  onSortChange: (next: string) => void;
  selected: ReadonlySet<string>;
  onSelectedChange: (next: Set<string>) => void;
  onOpenRow?: (row: T) => void;
  rowActions?: (row: T) => ReactNode;
  status: GridStatus;
  /** Shown in place of rows when `status` is `error`. */
  error?: { title: string; detail: string } | null;
  onRetry?: () => void;
  emptyTitle?: string;
  emptyDescription?: string;
  emptyAction?: ReactNode;
  density?: Density;
  /** True while a background refetch is in flight over already-shown rows. */
  refreshing?: boolean;
}

/** `-created` → `{ key: "created", direction: "desc" }`. */
function parseSort(sort: string): { key: string; direction: "asc" | "desc" } | null {
  const first = sort.split(",")[0]?.trim();
  if (!first) return null;
  return first.startsWith("-") ? { key: first.slice(1), direction: "desc" } : { key: first, direction: "asc" };
}

/**
 * The records grid: a dense, edge-to-edge, virtualized database table.
 *
 * Rows are windowed with `@tanstack/react-virtual`, so a 500-row page costs
 * about as much to render as a 25-row one. Sorting is *only* a server
 * concern — the grid renders `rows` in the order they arrived and reports a
 * `sort` expression back up; there is deliberately no client-side re-sort,
 * which would silently reorder one page of a much larger result set and
 * disagree with the next page.
 */
export function RecordsGrid<T>({
  label,
  rows,
  columns,
  getRowId,
  sort,
  onSortChange,
  selected,
  onSelectedChange,
  onOpenRow,
  rowActions,
  status,
  error,
  onRetry,
  emptyTitle = "No records yet",
  emptyDescription = "Rows added here — or written through the API — show up in this grid.",
  emptyAction,
  density = "comfortable",
  refreshing = false,
}: RecordsGridProps<T>) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<{ row: number; col: number } | null>(null);
  const [editing, setEditing] = useState<{ row: number; col: number } | null>(null);
  const anchorRef = useRef<number | null>(null);
  const rowHeight = ROW_HEIGHT[density];
  const current = parseSort(sort);

  const template = useMemo(
    () =>
      [
        `${SELECT_WIDTH}px`,
        ...columns.map((c) => `${c.width}px`),
        rowActions ? `${ACTIONS_WIDTH}px` : null,
      ]
        .filter(Boolean)
        .join(" "),
    [columns, rowActions],
  );

  const totalWidth =
    SELECT_WIDTH + columns.reduce((sum, c) => sum + c.width, 0) + (rowActions ? ACTIONS_WIDTH : 0);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: OVERSCAN,
  });

  // A new page, filter or sort resets the cursor: keeping row 12 selected
  // when row 12 is now a different record is worse than losing the cursor.
  useEffect(() => {
    setActive(null);
    setEditing(null);
    anchorRef.current = null;
  }, [rows]);

  const focusCell = useCallback((row: number, col: number) => {
    const el = scrollRef.current?.querySelector<HTMLElement>(`[data-cell="${row}:${col}"]`);
    el?.focus({ preventScroll: true });
  }, []);

  const moveTo = useCallback(
    (row: number, col: number) => {
      const nextRow = Math.max(0, Math.min(rows.length - 1, row));
      const nextCol = Math.max(0, Math.min(columns.length - 1, col));
      setActive({ row: nextRow, col: nextCol });
      setEditing(null);
      virtualizer.scrollToIndex(nextRow, { align: "auto" });
      // The row may not exist in the DOM until the virtualizer flushes.
      requestAnimationFrame(() => focusCell(nextRow, nextCol));
    },
    [rows.length, columns.length, virtualizer, focusCell],
  );

  const toggleRow = useCallback(
    (index: number, shiftKey: boolean) => {
      const next = new Set(selected);
      const id = getRowId(rows[index]!);
      const anchor = anchorRef.current;
      if (shiftKey && anchor !== null) {
        const [from, to] = anchor <= index ? [anchor, index] : [index, anchor];
        const turningOn = !selected.has(id);
        for (let i = from; i <= to; i++) {
          const rowId = getRowId(rows[i]!);
          if (turningOn) next.add(rowId);
          else next.delete(rowId);
        }
      } else {
        if (next.has(id)) next.delete(id);
        else next.add(id);
        anchorRef.current = index;
      }
      onSelectedChange(next);
    },
    [selected, rows, getRowId, onSelectedChange],
  );

  function handleHeaderSort(column: GridColumn<T>) {
    if (!column.sortKey) return;
    const key = column.sortKey;
    if (current?.key !== key) onSortChange(key);
    else if (current.direction === "asc") onSortChange(`-${key}`);
    else onSortChange("");
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (editing) return;
    const cursor = active ?? { row: 0, col: 0 };
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        moveTo(cursor.row + (active ? 1 : 0), cursor.col);
        break;
      case "ArrowUp":
        event.preventDefault();
        moveTo(cursor.row - (active ? 1 : 0), cursor.col);
        break;
      case "ArrowRight":
        event.preventDefault();
        moveTo(cursor.row, cursor.col + (active ? 1 : 0));
        break;
      case "ArrowLeft":
        event.preventDefault();
        moveTo(cursor.row, cursor.col - (active ? 1 : 0));
        break;
      case "Home":
        event.preventDefault();
        moveTo(event.ctrlKey || event.metaKey ? 0 : cursor.row, 0);
        break;
      case "End":
        event.preventDefault();
        moveTo(event.ctrlKey || event.metaKey ? rows.length - 1 : cursor.row, columns.length - 1);
        break;
      case "PageDown":
        event.preventDefault();
        moveTo(cursor.row + 20, cursor.col);
        break;
      case "PageUp":
        event.preventDefault();
        moveTo(cursor.row - 20, cursor.col);
        break;
      case "Enter": {
        if (!active) break;
        event.preventDefault();
        if (columns[active.col]?.editable) setEditing(active);
        else if (onOpenRow) onOpenRow(rows[active.row]!);
        break;
      }
      case " ": {
        if (!active) break;
        event.preventDefault();
        toggleRow(active.row, event.shiftKey);
        break;
      }
      case "Escape":
        setEditing(null);
        break;
      default:
        break;
    }
  }

  const allOnPageSelected = rows.length > 0 && rows.every((row) => selected.has(getRowId(row)));
  const someOnPageSelected = !allOnPageSelected && rows.some((row) => selected.has(getRowId(row)));

  const headerCellClass =
    "flex h-full items-center gap-1 overflow-hidden whitespace-nowrap border-r border-border px-2 text-xs font-medium text-muted-foreground last:border-r-0";

  const overlay =
    status === "error" ? (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon" className="text-destructive">
            <TriangleAlert />
          </EmptyMedia>
          <EmptyTitle>{error?.title ?? "Couldn't load these records"}</EmptyTitle>
          <EmptyDescription>{error?.detail ?? "The server didn't answer that query."}</EmptyDescription>
        </EmptyHeader>
        {onRetry ? (
          <Button size="sm" variant="outline" onClick={onRetry}>
            Try again
          </Button>
        ) : null}
      </Empty>
    ) : status === "ready" && rows.length === 0 ? (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Rows3 />
          </EmptyMedia>
          <EmptyTitle>{emptyTitle}</EmptyTitle>
          <EmptyDescription>{emptyDescription}</EmptyDescription>
        </EmptyHeader>
        {emptyAction}
      </Empty>
    ) : null;

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <div
        ref={scrollRef}
        role="grid"
        aria-label={label}
        aria-rowcount={rows.length}
        aria-busy={status === "loading" || undefined}
        tabIndex={-1}
        onKeyDown={handleKeyDown}
        className="relative min-h-0 flex-1 overflow-auto outline-none"
      >
        <div style={{ minWidth: `${totalWidth}px` }}>
          {/* Header ---------------------------------------------------- */}
          <div
            role="row"
            style={{ gridTemplateColumns: template }}
            className="sticky top-0 z-sticky grid h-control-sm border-b border-border bg-surface-sunken/95 backdrop-blur-sm"
          >
            <div role="columnheader" className={cn(headerCellClass, "justify-center px-0")}>
              <Checkbox
                aria-label={allOnPageSelected ? "Clear selection" : "Select all rows on this page"}
                checked={allOnPageSelected ? true : someOnPageSelected ? "indeterminate" : false}
                disabled={rows.length === 0}
                onCheckedChange={(next) => {
                  if (next === true) onSelectedChange(new Set([...selected, ...rows.map(getRowId)]));
                  else {
                    const cleared = new Set(selected);
                    for (const row of rows) cleared.delete(getRowId(row));
                    onSelectedChange(cleared);
                  }
                  anchorRef.current = null;
                }}
              />
            </div>
            {columns.map((column) => {
              const sorted = column.sortKey && current?.key === column.sortKey ? current.direction : null;
              return (
                <div
                  key={column.id}
                  role="columnheader"
                  aria-sort={
                    column.sortKey ? (sorted === "asc" ? "ascending" : sorted === "desc" ? "descending" : "none") : undefined
                  }
                  className={headerCellClass}
                >
                  {column.sortKey ? (
                    <button
                      type="button"
                      onClick={() => handleHeaderSort(column)}
                      title={`Sort by ${column.header}`}
                      className="group/sort flex min-w-0 items-center gap-1 font-mono transition-colors hover:text-foreground"
                    >
                      <span className="truncate">{column.header}</span>
                      {sorted === "asc" ? (
                        <ArrowUp className="size-3 shrink-0 text-foreground" />
                      ) : sorted === "desc" ? (
                        <ArrowDown className="size-3 shrink-0 text-foreground" />
                      ) : (
                        <ChevronsUpDown className="size-3 shrink-0 opacity-0 transition-opacity group-hover/sort:opacity-60" />
                      )}
                    </button>
                  ) : (
                    <span className="truncate font-mono">{column.header}</span>
                  )}
                </div>
              );
            })}
            {rowActions ? <div role="columnheader" className={headerCellClass} aria-label="Row actions" /> : null}
          </div>

          {/* Body ------------------------------------------------------ */}
          {status === "loading" ? (
            <div className="flex flex-col">
              {Array.from({ length: 12 }).map((_, i) => (
                <div
                  key={i}
                  style={{ gridTemplateColumns: template, height: rowHeight }}
                  className="grid items-center border-b border-border"
                >
                  <div />
                  {columns.map((column) => (
                    <div key={column.id} className="border-r border-border px-2 last:border-r-0">
                      <Skeleton className="h-3" style={{ width: `${40 + ((i * 17 + column.width) % 45)}%` }} />
                    </div>
                  ))}
                  {rowActions ? <div /> : null}
                </div>
              ))}
            </div>
          ) : overlay ? null : (
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((virtualRow) => {
                const row = rows[virtualRow.index]!;
                const id = getRowId(row);
                const isSelected = selected.has(id);
                return (
                  <div
                    key={id}
                    role="row"
                    aria-rowindex={virtualRow.index + 1}
                    aria-selected={isSelected}
                    data-selected={isSelected || undefined}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      height: virtualRow.size,
                      transform: `translateY(${virtualRow.start}px)`,
                      gridTemplateColumns: template,
                    }}
                    className="group/row grid border-b border-border transition-colors hover:bg-accent/40 data-[selected]:bg-primary/[0.07]"
                  >
                    <div className="flex items-center justify-center border-r border-border">
                      <Checkbox
                        aria-label={`Select record ${id}`}
                        checked={isSelected}
                        onClick={(event) => {
                          event.stopPropagation();
                          toggleRow(virtualRow.index, event.shiftKey);
                        }}
                        className="opacity-60 transition-opacity group-hover/row:opacity-100 data-[state=checked]:opacity-100"
                      />
                    </div>
                    {columns.map((column, colIndex) => {
                      const isActive = active?.row === virtualRow.index && active.col === colIndex;
                      const isEditing = editing?.row === virtualRow.index && editing.col === colIndex;
                      return (
                        <div
                          key={column.id}
                          role="gridcell"
                          data-cell={`${virtualRow.index}:${colIndex}`}
                          tabIndex={isActive ? 0 : -1}
                          onFocus={() => setActive({ row: virtualRow.index, col: colIndex })}
                          onMouseDown={() => setActive({ row: virtualRow.index, col: colIndex })}
                          className={cn(
                            "flex min-w-0 items-center overflow-hidden border-r border-border px-2 outline-none last:border-r-0",
                            isActive && "ring-1 ring-inset ring-ring",
                          )}
                        >
                          {column.cell(row, {
                            editing: isEditing,
                            active: isActive,
                            beginEdit: () => setEditing({ row: virtualRow.index, col: colIndex }),
                            endEdit: () => {
                              setEditing(null);
                              focusCell(virtualRow.index, colIndex);
                            },
                          })}
                        </div>
                      );
                    })}
                    {rowActions ? <div className="flex items-center justify-center">{rowActions(row)}</div> : null}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* A refetch over rows already on screen shouldn't blank them out;
            a hairline progress bar says "working" without moving anything. */}
        {refreshing && status === "ready" ? (
          <div className="pointer-events-none sticky bottom-0 left-0 h-px w-full bg-primary/40" />
        ) : null}
      </div>

      {/* Empty and error states sit over the scroller, not inside it: a
          message centred on a 3000px-wide scroll canvas is a message
          nobody can see. */}
      {overlay ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-0 top-control-sm flex items-start justify-center overflow-hidden px-page pt-12">
          <div className="pointer-events-auto">{overlay}</div>
        </div>
      ) : null}
    </div>
  );
}
