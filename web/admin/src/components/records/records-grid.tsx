import { useCallback, type ReactNode } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { useSortableRows, type SortState } from "@/components/interior/sortable-table";

export type GridColumn<T> = {
  id: string;
  header: string;
  /** css width; omit to let the column hug its content like a real db client */
  width?: string;
  sortable?: boolean;
  value?: (row: T) => string | number | null | undefined;
  cell: (row: T) => ReactNode;
};

/** A real, dense, edge-to-edge database grid — full vertical + horizontal
 * gridlines, monospace, content-sized columns — instead of a generic
 * rounded "list" card. Modeled on Drizzle Studio / Supabase's table editor. */
export function RecordsGrid<T>({
  rows,
  columns,
  getRowId,
  sort,
  onSortChange,
  label,
}: {
  rows: T[];
  columns: GridColumn<T>[];
  getRowId: (row: T) => string;
  sort?: SortState | null;
  onSortChange?: (next: SortState | null) => void;
  label: string;
}) {
  const getValue = useCallback(
    (row: T, columnId: string) => {
      const column = columns.find((c) => c.id === columnId);
      return column?.value ? column.value(row) : null;
    },
    [columns],
  );

  const { sort: current, ordered, toggle, ariaSort } = useSortableRows<T>({
    rows,
    getRowId,
    getValue,
    sort,
    onSortChange,
  });

  return (
    <div className="w-full overflow-auto border border-border">
      <table role="table" aria-label={label} className="w-full border-collapse text-left font-mono text-[12.5px]">
        <thead>
          <tr role="row">
            {columns.map((column) => (
              <th
                key={column.id}
                role="columnheader"
                aria-sort={column.sortable ? ariaSort(column.id) : undefined}
                style={{ width: column.width, minWidth: column.width }}
                className="sticky top-0 z-10 whitespace-nowrap border-b border-r border-border bg-secondary/70 px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground last:border-r-0"
              >
                {column.sortable ? (
                  <button
                    type="button"
                    onClick={() => toggle(column.id)}
                    className="flex items-center gap-1 normal-case tracking-normal hover:text-foreground"
                  >
                    {column.header}
                    {current?.columnId === column.id ? (
                      current.direction === "asc" ? (
                        <ChevronUp className="size-3" />
                      ) : (
                        <ChevronDown className="size-3" />
                      )
                    ) : null}
                  </button>
                ) : (
                  column.header
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr role="row">
              <td colSpan={columns.length} className="px-3 py-16 text-center font-sans text-[13px] text-muted-foreground">
                No records yet
              </td>
            </tr>
          ) : (
            ordered.map(({ id, row }) => (
              <tr key={id} role="row" className="group hover:bg-accent/40">
                {columns.map((column) => (
                  <td
                    key={column.id}
                    role="cell"
                    style={{ width: column.width, minWidth: column.width }}
                    className="border-b border-r border-border px-3 py-1.5 align-middle last:border-r-0"
                  >
                    {column.cell(row)}
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}
