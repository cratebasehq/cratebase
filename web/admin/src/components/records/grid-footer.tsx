import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Pagination } from "@/components/ui/pagination";

/** Page controls plus the row range. With `skipTotal` on there is no last
 * page to jump to, so it degrades to prev/next rather than lying about a
 * page count. */
export function GridFooter({
  page,
  perPage,
  rowsOnPage,
  totalItems,
  totalPages,
  onPageChange,
}: {
  page: number;
  perPage: number;
  rowsOnPage: number;
  totalItems: number;
  totalPages: number;
  onPageChange: (next: number) => void;
}) {
  const first = rowsOnPage === 0 ? 0 : (page - 1) * perPage + 1;
  const last = (page - 1) * perPage + rowsOnPage;
  const known = totalItems >= 0;

  return (
    <div className="flex h-control-lg shrink-0 items-center justify-between gap-3 border-t border-border px-page">
      <span className="font-tabular text-xs text-muted-foreground">
        {rowsOnPage === 0
          ? "No rows"
          : known
            ? `${first.toLocaleString()}–${last.toLocaleString()} of ${totalItems.toLocaleString()}`
            : `${first.toLocaleString()}–${last.toLocaleString()}`}
      </span>

      {known && totalPages > 1 ? (
        <Pagination count={totalPages} page={page} onPageChange={onPageChange} label="Records pages" />
      ) : !known ? (
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            className="h-control-sm gap-1"
            disabled={page <= 1}
            onClick={() => onPageChange(page - 1)}
          >
            <ChevronLeft className="size-3.5" />
            Prev
          </Button>
          <span className="font-tabular text-xs text-muted-foreground">page {page}</span>
          <Button
            variant="ghost"
            size="sm"
            className="h-control-sm gap-1"
            disabled={rowsOnPage < perPage}
            onClick={() => onPageChange(page + 1)}
          >
            Next
            <ChevronRight className="size-3.5" />
          </Button>
        </div>
      ) : null}
    </div>
  );
}
