"use client"

import { useCallback, useEffect, useRef, useState } from "react"
import { ChevronLeft, ChevronRight } from "lucide-react"

import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"

type PaginationItem = number | "gap-l" | "gap-r"

const range = (from: number, to: number) =>
  Array.from({ length: to - from + 1 }, (_, i) => from + i)

/* Boundaries always show; siblings flank the current page. `total` is the
 * widest run that still fits before an ellipsis earns its place. */
function paginate(
  page: number,
  count: number,
  siblings: number,
  boundaries: number
): PaginationItem[] {
  const total = 2 * boundaries + 2 * siblings + 3
  if (count <= total) return range(1, count)

  if (page < boundaries + siblings + 2) {
    const head = range(1, 2 * siblings + boundaries + 2)
    return [...head, "gap-r", ...range(count - boundaries + 1, count)]
  }
  if (page > count - boundaries - siblings - 1) {
    const tail = range(count - 2 * siblings - boundaries - 1, count)
    return [...range(1, boundaries), "gap-l", ...tail]
  }
  return [
    ...range(1, boundaries),
    "gap-l",
    ...range(page - siblings, page + siblings),
    "gap-r",
    ...range(count - boundaries + 1, count),
  ]
}

type PaginationProps = {
  count: number
  page?: number
  defaultPage?: number
  siblings?: number
  boundaries?: number
  onPageChange?: (page: number) => void
  label?: string
  className?: string
}

function Pagination({
  count,
  page,
  defaultPage = 1,
  siblings = 1,
  boundaries = 1,
  onPageChange,
  label = "Pagination",
  className,
}: PaginationProps) {
  const clampTo = useCallback(
    (value: number) => Math.min(Math.max(1, value), Math.max(1, count)),
    [count]
  )

  const [internal, setInternal] = useState(() => clampTo(defaultPage))
  const controlled = page !== undefined
  const current = clampTo(controlled ? page : internal)

  const emit = useRef(onPageChange)
  emit.current = onPageChange

  const goTo = (value: number) => {
    const next = clampTo(value)
    if (next === current) return
    if (!controlled) setInternal(next)
    emit.current?.(next)
  }

  // Announcing on a delay keeps rapid paging from flooding the live region.
  const [spoken, setSpoken] = useState("")
  useEffect(() => {
    const id = setTimeout(
      () => setSpoken(`Page ${current} of ${Math.max(1, count)}`),
      500
    )
    return () => clearTimeout(id)
  }, [current, count])

  return (
    <nav
      data-slot="pagination"
      aria-label={label}
      className={cn("inline-flex items-center gap-1", className)}
    >
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Previous page"
        disabled={current <= 1}
        onClick={() => goTo(current - 1)}
      >
        <ChevronLeft />
      </Button>

      {paginate(current, count, siblings, boundaries).map((item) =>
        typeof item === "number" ? (
          <Button
            key={`page-${item}`}
            variant={item === current ? "secondary" : "ghost"}
            size="sm"
            className="min-w-7 px-1.5 font-tabular"
            aria-label={`Page ${item}`}
            aria-current={item === current ? "page" : undefined}
            onClick={() => goTo(item)}
          >
            {item}
          </Button>
        ) : (
          <span
            key={item}
            aria-hidden
            className="flex size-7 items-center justify-center text-xs text-muted-foreground"
          >
            &hellip;
          </span>
        )
      )}

      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Next page"
        disabled={current >= count}
        onClick={() => goTo(current + 1)}
      >
        <ChevronRight />
      </Button>

      <span role="status" className="sr-only">
        {spoken}
      </span>
    </nav>
  )
}

export { Pagination, paginate }
export type { PaginationProps, PaginationItem }
