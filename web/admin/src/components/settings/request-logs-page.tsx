import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { cb } from "@/lib/api";
import { settingsLogsRoute } from "@/routes/settings-logs";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Pagination } from "@/components/ui/pagination";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ListTree } from "lucide-react";

/**
 * A log row as the server writes it, which is PocketBase's shape: the
 * request's details live under `data`, not on the row. The row itself
 * only carries the level, the rendered message and the timestamp.
 *
 * `level` is 0 for a 2xx and 8 for a failure, and a failing row also
 * carries `data.error`.
 */
type RequestLogEntry = {
  id: string;
  level: number;
  message: string;
  created: string;
  data?: {
    type?: string;
    method?: string;
    url?: string;
    status?: number;
    execTime?: number;
    auth?: string;
    userIP?: string;
    remoteIP?: string;
    referer?: string;
    userAgent?: string;
    error?: string;
  };
};

type RequestLogsResult = {
  page: number;
  perPage: number;
  totalItems: number;
  totalPages: number;
  items: RequestLogEntry[];
};

function statusVariant(status: number): "default" | "secondary" | "destructive" {
  if (status >= 500) return "destructive";
  if (status >= 400) return "secondary";
  return "default";
}

/** `data.auth` is the empty string for a guest, not absent. */
function callerLabel(entry: RequestLogEntry): string {
  const auth = entry.data?.auth;
  return auth && auth.length > 0 ? auth : "anonymous";
}

/** The path, without the origin the server records it with. */
function pathOf(entry: RequestLogEntry): string {
  const url = entry.data?.url;
  if (!url) return entry.message ?? "";
  try {
    return new URL(url, "http://localhost").pathname + new URL(url, "http://localhost").search;
  } catch {
    return url;
  }
}

/** Superuser-only view over `_request_logs`, the bounded history the
 * `request_log` middleware writes on every `/api/*` call. Read-only —
 * there's nothing to edit here, just something to search. */
export function RequestLogsPage() {
  const urlSearch = settingsLogsRoute.useSearch();
  const navigate = settingsLogsRoute.useNavigate();
  const page = urlSearch.page ?? 1;
  const filter = urlSearch.filter ?? "";
  const [filterInput, setFilterInput] = useState(filter);

  useEffect(() => {
    setFilterInput(filter);
  }, [filter]);

  const { data, isLoading } = useQuery({
    queryKey: ["request-logs", page, filter],
    queryFn: () =>
      cb.send<RequestLogsResult>("/api/logs", {
        method: "GET",
        query: { page, perPage: 30, filter: filter || undefined },
      }),
    placeholderData: (previous) => previous,
  });

  function submitFilter(e: React.FormEvent) {
    e.preventDefault();
    void navigate({ search: { page: undefined, filter: filterInput.trim() || undefined }, replace: true });
  }

  return (
    <div className="flex flex-col gap-4 p-6">
      <form onSubmit={submitFilter} className="flex max-w-sm items-center gap-2">
        <Input
          value={filterInput}
          onChange={(e) => setFilterInput(e.target.value)}
          placeholder="Filter by path…"
          aria-label="Filter request logs by path"
        />
      </form>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-[90px]">Method</TableHead>
            <TableHead>Path</TableHead>
            <TableHead className="w-[80px]">Status</TableHead>
            <TableHead className="w-[100px]">Duration</TableHead>
            <TableHead>Caller</TableHead>
            <TableHead className="w-[180px]">Time</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {(data?.items ?? []).map((entry) => (
            <TableRow key={entry.id}>
              <TableCell className="font-mono text-xs">{entry.data?.method ?? "—"}</TableCell>
              <TableCell className="font-mono text-xs">{pathOf(entry)}</TableCell>
              <TableCell>
                <Badge variant={statusVariant(entry.data?.status ?? 0)}>{entry.data?.status ?? "—"}</Badge>
              </TableCell>
              <TableCell className="text-xs text-muted-foreground">
                  {entry.data?.execTime != null ? `${entry.data.execTime.toFixed(2)}ms` : "—"}
                </TableCell>
              <TableCell className="text-xs text-muted-foreground">{callerLabel(entry)}</TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {new Date(entry.created).toLocaleString()}
              </TableCell>
            </TableRow>
          ))}
          {isLoading ? (
            Array.from({ length: 5 }).map((_, i) => (
              <TableRow key={i}>
                <TableCell colSpan={6}>
                  <Skeleton className="h-row w-full" />
                </TableCell>
              </TableRow>
            ))
          ) : null}
          {!isLoading && (data?.items.length ?? 0) === 0 ? (
            <TableRow>
              <TableCell colSpan={6} className="py-8">
                <Empty>
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <ListTree />
                    </EmptyMedia>
                    <EmptyTitle>No requests logged yet</EmptyTitle>
                    <EmptyDescription>
                      Every call to <code className="font-mono">/api/*</code> lands here as it happens.
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>

      {data && data.totalPages > 1 ? (
        <div className="flex justify-center py-2">
          <Pagination
            count={data.totalPages}
            page={page}
            onPageChange={(next) =>
              void navigate({ search: (prev) => ({ ...prev, page: next > 1 ? next : undefined }), replace: true })
            }
            label="Request log pages"
          />
        </div>
      ) : null}
    </div>
  );
}
