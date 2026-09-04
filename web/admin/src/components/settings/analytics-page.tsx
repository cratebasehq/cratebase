import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Bar, BarChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { cb } from "@/lib/api";
import { cn } from "@/lib/utils";
import { settingsAnalyticsRoute } from "@/routes/settings-analytics";
import { Pagination } from "@/components/ui/pagination";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ChartNoAxesColumn } from "lucide-react";

/** The window the stat cards and chart summarize over. Computing an exact
 * all-time unique-path count or top referrer would mean scanning every row
 * in `analytics` on every page load; 30 days is enough to be useful and
 * cheap to fetch in one request via `fields=path,referrer,created`. The
 * "Total pageviews" card is the one number that stays exact and all-time —
 * it only costs a `getList(1, 1)` for `totalItems`, no row data at all. */
const WINDOW_DAYS = 30;

/** A pageview as the beacon writes it (`web/beacon/beacon.js`) plus the
 * server's own `id`/`created`. `referrer` and `visitorHash` are optional —
 * the beacon never sends `visitorHash` (see the module doc below), so it is
 * always empty here today. */
type AnalyticsEntry = {
  id: string;
  url: string;
  path: string;
  referrer: string;
  created: string;
};

function StatCard({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5 rounded-lg border border-border bg-card px-3 py-2.5">
      <span className="text-2xs uppercase tracking-wider text-muted-foreground/70">{label}</span>
      <span className="truncate font-tabular text-2xl leading-none tracking-tight">{value}</span>
      {hint ? <span className="truncate text-2xs text-muted-foreground">{hint}</span> : null}
    </div>
  );
}

/** Daily pageview counts over {@link WINDOW_DAYS}, one bar per day —
 * mirrors `RequestLogsChart`'s shape (`request-logs-page.tsx`), the
 * existing pattern for "a small time-series above a table" in this
 * dashboard. */
function DailyChart({ entries }: { entries: AnalyticsEntry[] }) {
  const chartData = useMemo(() => {
    const counts = new Map<string, number>();
    for (const entry of entries) {
      const key = entry.created.slice(0, 10);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    const days: { day: string; total: number }[] = [];
    for (let i = WINDOW_DAYS - 1; i >= 0; i--) {
      const d = new Date(Date.now() - i * 24 * 60 * 60 * 1000);
      const key = d.toISOString().slice(0, 10);
      days.push({
        day: d.toLocaleDateString(undefined, { month: "short", day: "numeric" }),
        total: counts.get(key) ?? 0,
      });
    }
    return days;
  }, [entries]);

  if (entries.length === 0) return null;

  return (
    <div className="h-32 w-full rounded-lg border border-border bg-card p-2">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={chartData} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
          <XAxis
            dataKey="day"
            tick={{ fontSize: 10 }}
            tickLine={false}
            axisLine={false}
            interval="preserveStartEnd"
            className="fill-muted-foreground"
          />
          <YAxis hide allowDecimals={false} />
          <Tooltip
            cursor={{ fill: "var(--color-accent)" }}
            contentStyle={{
              background: "var(--color-popover)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-lg)",
              fontSize: 12,
            }}
            labelFormatter={(label) => label}
            formatter={(value) => [`${value} pageview${value === 1 ? "" : "s"}`, undefined]}
          />
          <Bar dataKey="total" radius={[2, 2, 0, 0]} className="fill-primary" />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

/**
 * Self-hosted, cookie-free web analytics — the dashboard half of the
 * beacon in `web/beacon/beacon.js`. Everything here is an ordinary query
 * against `analytics` through the same records API any other client
 * would use (list + filter + sort); there is no bespoke analytics
 * endpoint on the server.
 *
 * **Unique-visitor counting is not implemented.** The beacon never sends
 * anything resembling a persistent identifier (no cookie, no
 * localStorage, no fingerprint) — that is the point of a privacy-first
 * beacon. A safe substitute (e.g. a rotating daily hash of IP+User-Agent)
 * has to be computed server-side, which would mean a bespoke ingestion
 * endpoint instead of "the generic records API already allows this
 * write" — out of scope for this pass. What this page reports instead:
 * raw pageviews, distinct paths visited, and referrers, all of which are
 * honest counts of what the beacon actually sends.
 */
export function AnalyticsPage() {
  const urlSearch = settingsAnalyticsRoute.useSearch();
  const navigate = settingsAnalyticsRoute.useNavigate();
  const page = urlSearch.page ?? 1;

  const totalQuery = useQuery({
    queryKey: ["analytics", "total"],
    queryFn: () => cb.collection("analytics").getList<AnalyticsEntry>(1, 1, { requestKey: null }),
  });

  const windowQuery = useQuery({
    queryKey: ["analytics", "window"],
    queryFn: () =>
      cb.collection("analytics").getFullList<AnalyticsEntry>({
        filter: `created >= "${new Date(Date.now() - WINDOW_DAYS * 24 * 60 * 60 * 1000).toISOString().replace("T", " ").replace("Z", ".000Z")}"`,
        sort: "-created",
        fields: "id,path,referrer,created",
        batch: 500,
        requestKey: null,
      }),
  });

  const tableQuery = useQuery({
    queryKey: ["analytics", "recent", page],
    queryFn: () =>
      cb.collection("analytics").getList<AnalyticsEntry>(page, 30, { sort: "-created", requestKey: null }),
    placeholderData: (previous) => previous,
  });

  const windowEntries = windowQuery.data ?? [];

  const uniquePaths = useMemo(() => new Set(windowEntries.map((e) => e.path)).size, [windowEntries]);

  const topReferrer = useMemo(() => {
    const counts = new Map<string, number>();
    for (const entry of windowEntries) {
      const ref = entry.referrer ? refererHost(entry.referrer) : "";
      if (!ref) continue;
      counts.set(ref, (counts.get(ref) ?? 0) + 1);
    }
    let best: [string, number] | null = null;
    for (const pair of counts) if (!best || pair[1] > best[1]) best = pair;
    return best;
  }, [windowEntries]);

  const loading = totalQuery.isLoading || windowQuery.isLoading;

  return (
    <div className="flex flex-col gap-4 p-6">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <StatCard
          label="Total pageviews"
          value={loading ? "…" : (totalQuery.data?.totalItems ?? 0).toLocaleString()}
          hint="all time"
        />
        <StatCard
          label="Unique paths"
          value={loading ? "…" : uniquePaths.toLocaleString()}
          hint={`last ${WINDOW_DAYS} days`}
        />
        <StatCard
          label="Top referrer"
          value={loading ? "…" : (topReferrer?.[0] ?? "Direct")}
          hint={topReferrer ? `${topReferrer[1]} visit${topReferrer[1] === 1 ? "" : "s"}, last ${WINDOW_DAYS} days` : `last ${WINDOW_DAYS} days`}
        />
      </div>

      <DailyChart entries={windowEntries} />

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Path</TableHead>
            <TableHead>Referrer</TableHead>
            <TableHead className="w-[180px]">Time</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {(tableQuery.data?.items ?? []).map((entry) => (
            <TableRow key={entry.id}>
              <TableCell className="font-mono text-xs">{entry.path || "/"}</TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {entry.referrer ? refererHost(entry.referrer) : "Direct"}
              </TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {new Date(entry.created).toLocaleString()}
              </TableCell>
            </TableRow>
          ))}
          {tableQuery.isLoading
            ? Array.from({ length: 5 }).map((_, i) => (
                <TableRow key={i}>
                  <TableCell colSpan={3}>
                    <Skeleton className={cn("h-row w-full")} />
                  </TableCell>
                </TableRow>
              ))
            : null}
          {!tableQuery.isLoading && (tableQuery.data?.items.length ?? 0) === 0 ? (
            <TableRow>
              <TableCell colSpan={3} className="py-8">
                <Empty>
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <ChartNoAxesColumn />
                    </EmptyMedia>
                    <EmptyTitle>No pageviews recorded yet</EmptyTitle>
                    <EmptyDescription>
                      Drop <code className="font-mono">web/beacon/beacon.js</code> in a{" "}
                      <code className="font-mono">&lt;script&gt;</code> tag on a site to start seeing pageviews here.
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>

      {tableQuery.data && tableQuery.data.totalPages > 1 ? (
        <div className="flex justify-center py-2">
          <Pagination
            count={tableQuery.data.totalPages}
            page={page}
            onPageChange={(next) =>
              void navigate({ search: (prev) => ({ ...prev, page: next > 1 ? next : undefined }), replace: true })
            }
            label="Analytics pages"
          />
        </div>
      ) : null}
    </div>
  );
}

/** `document.referrer` is a full URL; the host is what's actually useful
 * to compare across visits ("google.com", not the specific search-result
 * query string). Falls back to the raw value for anything that fails to
 * parse as a URL (e.g. an already-bare host someone wrote by hand). */
function refererHost(referrer: string): string {
  try {
    return new URL(referrer).host || referrer;
  } catch {
    return referrer;
  }
}
