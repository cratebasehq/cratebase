import { useCallback, useMemo, useState } from "react"

type SortDirection = "asc" | "desc"

type SortState = { columnId: string; direction: SortDirection }

type OrderedRow<T> = { id: string; row: T; index: number }

type UseSortableRowsOptions<T> = {
  rows: T[]
  getRowId: (row: T) => string
  getValue: (row: T, columnId: string) => string | number | null | undefined
  sort?: SortState | null
  defaultSort?: SortState | null
  onSortChange?: (next: SortState | null) => void
  restoreOriginal?: boolean
}

function useSortableRows<T>({
  rows,
  getRowId,
  getValue,
  sort,
  defaultSort = null,
  onSortChange,
  restoreOriginal = true,
}: UseSortableRowsOptions<T>) {
  const [internal, setInternal] = useState<SortState | null>(defaultSort)

  const controlled = sort !== undefined
  const current = controlled ? sort : internal

  const collator = useMemo(
    () => new Intl.Collator("en", { numeric: true, sensitivity: "base" }),
    []
  )

  const ordered = useMemo<OrderedRow<T>[]>(() => {
    const base = rows.map((row, i) => ({ id: getRowId(row), row, i }))

    if (current) {
      const dir = current.direction === "asc" ? 1 : -1
      base.sort((x, y) => {
        const a = getValue(x.row, current.columnId)
        const b = getValue(y.row, current.columnId)
        const emptyA = a === null || a === undefined || a === ""
        const emptyB = b === null || b === undefined || b === ""
        // Blanks always sink, whichever way the column is pointing.
        if (emptyA || emptyB) {
          if (emptyA && emptyB) return x.i - y.i
          return emptyA ? 1 : -1
        }
        const d =
          typeof a === "number" && typeof b === "number"
            ? a - b
            : collator.compare(String(a), String(b))
        // Ties fall back to the source order, which keeps the sort stable.
        return d === 0 ? x.i - y.i : d * dir
      })
    }

    return base.map(({ id, row }, index) => ({ id, row, index }))
  }, [rows, current, getRowId, getValue, collator])

  const toggle = useCallback(
    (columnId: string) => {
      const next: SortState | null =
        !current || current.columnId !== columnId
          ? { columnId, direction: "asc" }
          : current.direction === "asc"
            ? { columnId, direction: "desc" }
            : restoreOriginal
              ? null
              : { columnId, direction: "asc" }

      if (!controlled) setInternal(next)
      onSortChange?.(next)
    },
    [current, controlled, onSortChange, restoreOriginal]
  )

  const ariaSort = useCallback(
    (columnId: string): "ascending" | "descending" | "none" =>
      current?.columnId === columnId
        ? current.direction === "asc"
          ? "ascending"
          : "descending"
        : "none",
    [current]
  )

  return { sort: current, ordered, toggle, ariaSort }
}

export { useSortableRows }
export type { SortState, SortDirection, OrderedRow, UseSortableRowsOptions }
