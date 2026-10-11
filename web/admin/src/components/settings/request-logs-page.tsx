import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Bar, BarChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { toast } from "sonner";
import { Copy, ListTree, RefreshCw } from "lucide-react";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { fillHourlyBuckets, type LogBucket } from "@/lib/log-stats";
import { settingsLogsRoute } from "@/routes/settings-logs";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Pagination } from "@/components/ui/pagination";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  DetailBody,
  DetailCallout,
  DetailList,
  DetailRow,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
  DetailTabPanel,
  DetailTabs,
} from "@/components/layout/detail-sheet";
import { CodeBlock } from "@/components/settings/detail-json";
import { TimeAgo } from "@/components/settings/time-ago";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatCard, StatusBadge, StatusDot, type Tone } from "@/components/layout/stat-card";
import {
  NumberSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  ToggleSetting,
} from "@/components/settings/settings-form";
import { useSaveShortcut } from "@/components/settings/use-save-shortcut";

const ITEM = SETTINGS_GROUPS.flatMap((g) => g.tabs).find((t) => t.legacyPath === "/settings/logs")!;

/** How often the live tail polls, in ms. */
const LIVE_INTERVAL = 3000;
/** A request slower than this (ms) counts as "slow" in the quick filter. */
const SLOW_MS = 500;

/** `slog`'s numeric levels, the usual convention — see
 * `crates/server/src/middleware/request_log.rs`'s `LEVEL_*` constants.
 * Request rows are always logged at `info`, so a `minLevel` above that
 * silences every request row (only application warn/error lines above
 * it still get through). */
const LOG_LEVELS = [
  { value: -4, label: "Debug", help: "Everything, including debug-only lines." },
  { value: 0, label: "Info (default)", help: "Every request, plus warnings and errors." },
  { value: 4, label: "Warn", help: "Only warnings and errors — no successful request rows." },
  { value: 8, label: "Error", help: "Only errors. Silences request rows entirely." },
];

const METHODS = ["GET", "POST", "PATCH", "PUT", "DELETE"] as const;

type StatusFilter = "all" | "errors" | "server" | "slow";

const STATUS_FILTERS: { value: StatusFilter; label: string }[] = [
  { value: "all", label: "All requests" },
  { value: "errors", label: "Errors (4xx, 5xx)" },
  { value: "server", label: "Server errors (5xx)" },
  { value: "slow", label: `Slow (≥ ${SLOW_MS} ms)` },
];

const STATUS_EXPR: Record<StatusFilter, string | null> = {
  all: null,
  errors: "data.status >= 400",
  server: "data.status >= 500",
  slow: `data.execTime >= ${SLOW_MS}`,
};

/** Free text is a path search (`message` is `"<METHOD> <path>"`); anything
 * that already looks like a filter expression is passed through untouched so
 * `data.status = 404 && data.method = "POST"` keeps working. */
function textExpression(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  if (/[=~<>!]|&&|\|\|/.test(trimmed)) return `(${trimmed})`;
  return `message ~ "${trimmed.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

/** The activity chart above the log table — hourly request volume for the
 * current filter, over a fixed last-24-hours grid. `GET /api/logs/stats`
 * only returns hours that actually have a row, so it is resampled with
 * `fillHourlyBuckets` — otherwise sparse traffic (often just one or two
 * real hours) collapses the chart to one or two bars stretched across
 * nearly the whole width instead of a real per-hour histogram. */
function useHourlyStats(filter: string, live: boolean) {
  return useQuery({
    queryKey: ["request-logs", "stats", filter],
    queryFn: () => cb.send<LogBucket[]>("/api/logs/stats", { method: "GET", query: { filter: filter || undefined } }),
    placeholderData: (previous) => previous,
    refetchInterval: live ? 15_000 : false,
  });
}

function RequestLogsChart({ stats }: { stats: LogBucket[] | undefined }) {
  const chartData = useMemo(
    () =>
      fillHourlyBuckets(stats ?? []).map((bucket) => ({
        ...bucket,
        label: new Date(bucket.date).toLocaleTimeString(undefined, { hour: "numeric" }),
      })),
    [stats],
  );

  return (
    <div
      className="h-28 w-full rounded-lg border border-border bg-card p-2 shadow-xs"
      role="img"
      aria-label="Requests per hour over the last 24 hours"
    >
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={chartData} margin={{ top: 4, right: 8, bottom: 0, left: 0 }} barCategoryGap="20%">
          <XAxis
            dataKey="label"
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
            formatter={(value) => [`${value} request${value === 1 ? "" : "s"}`, undefined]}
          />
          <Bar dataKey="total" radius={[2, 2, 0, 0]} maxBarSize={24} className="fill-primary" />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

/**
 * A log row as the server writes it, whose shape is: the
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

function statusTone(status: number): Tone {
  if (status >= 500) return "danger";
  if (status >= 400) return "warning";
  if (status >= 300) return "info";
  if (status >= 200) return "success";
  return "neutral";
}

const STATUS_TEXT: Record<number, string> = {
  200: "OK",
  201: "Created",
  204: "No content",
  301: "Moved",
  302: "Found",
  304: "Not modified",
  400: "Bad request",
  401: "Unauthorized",
  403: "Forbidden",
  404: "Not found",
  409: "Conflict",
  429: "Too many requests",
  500: "Server error",
  502: "Bad gateway",
  503: "Unavailable",
};

/** Latency band: colour + how full the little bar is (capped at one second). */
function latencyBand(ms: number): { tone: "ok" | "slow" | "bad"; fill: number } {
  return {
    tone: ms >= 1000 ? "bad" : ms >= SLOW_MS ? "slow" : "ok",
    fill: Math.min(100, Math.max(4, (ms / 1000) * 100)),
  };
}

function Latency({ ms }: { ms: number | undefined }) {
  if (ms == null) return <span className="text-muted-foreground">—</span>;
  const { tone, fill } = latencyBand(ms);
  return (
    <span className="inline-flex items-center gap-2">
      <span aria-hidden className="relative h-1 w-10 overflow-hidden rounded-full bg-muted">
        <span
          className={cn(
            "absolute inset-y-0 left-0 rounded-full",
            tone === "ok" && "bg-success/70",
            tone === "slow" && "bg-warning",
            tone === "bad" && "bg-destructive",
          )}
          style={{ width: `${fill}%` }}
        />
      </span>
      <span
        className={cn(
          "font-tabular w-16 text-right text-xs",
          tone === "ok" && "text-muted-foreground",
          tone === "slow" && "text-warning",
          tone === "bad" && "text-destructive",
        )}
      >
        {formatMs(ms)}
      </span>
    </span>
  );
}

function formatMs(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  if (ms >= 10) return `${Math.round(ms)} ms`;
  return `${ms.toFixed(2)} ms`;
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
    const parsed = new URL(url, "http://localhost");
    return parsed.pathname + parsed.search;
  } catch {
    return url;
  }
}

function timeOf(entry: RequestLogEntry): Date {
  return parseServerDate(entry.created);
}

/** Everything the server recorded about one request. The pager walks the
 * currently loaded page (newest first), so `prev` is the newer row. */
function RequestDetailSheet({
  entry,
  index,
  total,
  onClose,
  onStep,
}: {
  entry: RequestLogEntry | null;
  index: number;
  total: number;
  onClose: () => void;
  onStep: (delta: -1 | 1) => void;
}) {
  const { copy } = useCopyToClipboard();
  const json = useMemo(() => (entry ? JSON.stringify(entry, null, 2) : ""), [entry]);
  const [tab, setTab] = useState("overview");

  const d = entry?.data;
  const created = entry ? timeOf(entry) : null;
  const status = d?.status;
  return (
    <DetailSheet open={entry !== null} onOpenChange={(open) => !open && onClose()}>
      {entry ? (
        <>
          <DetailSheetHeader
            id={entry.id}
            pager={{
              index: index + 1,
              total,
              hasPrev: index > 0,
              hasNext: index < total - 1,
              onPrev: () => onStep(-1),
              onNext: () => onStep(1),
            }}
            menu={[
              { label: "Copy JSON", onSelect: () => void copy(json), icon: <Copy /> },
              { label: "Copy URL", onSelect: () => void copy(d?.url || pathOf(entry)), icon: <Copy /> },
            ]}
            title={
              <span className="break-all font-mono text-base">
                <span className="text-muted-foreground">{d?.method ?? "—"}</span> {pathOf(entry)}
              </span>
            }
            description={`${d?.method ?? ""} ${pathOf(entry)}`}
            status={
              <StatusBadge tone={statusTone(status ?? 0)}>
                {status ?? "—"}
                {status && STATUS_TEXT[status] ? ` ${STATUS_TEXT[status]}` : ""}
              </StatusBadge>
            }
            meta={[
              d?.execTime != null ? <span key="t" className="font-tabular font-mono">{formatMs(d.execTime)}</span> : "no timing",
              <TimeAgo key="a" value={entry.created} />,
              <span key="c" className="font-mono">{callerLabel(entry)}</span>,
            ]}
            actions={
              <Button size="sm" variant="outline" onClick={() => void copy(json)}>
                <Copy />
                Copy JSON
              </Button>
            }
          />
          <DetailTabs
            value={tab}
            onValueChange={setTab}
            tabs={[
              { value: "overview", label: "Overview" },
              { value: "raw", label: "Raw JSON" },
            ]}
          >
            <DetailTabPanel value="overview">
              <DetailBody>
                {d?.error ? (
                  <DetailCallout tone="danger" title={status ? `${status} ${STATUS_TEXT[status] ?? "Error"}` : "Error"}>
                    <span className="font-mono text-xs break-words">{d.error}</span>
                  </DetailCallout>
                ) : null}

                <DetailSection title="Request">
                  <DetailList>
                    <DetailRow label="Method">{d?.method ?? "—"}</DetailRow>
                    <DetailRow label="Path">
                      <span className="font-mono text-xs break-all">{pathOf(entry)}</span>
                    </DetailRow>
                    <DetailRow label="Full URL">
                      <span className="font-mono text-xs break-all">{d?.url || "—"}</span>
                    </DetailRow>
                    <DetailRow label="Referer">
                      <span className="font-mono text-xs break-all">{d?.referer || "—"}</span>
                    </DetailRow>
                  </DetailList>
                </DetailSection>

                <DetailSection title="Timing">
                  <DetailList>
                    <DetailRow label="Duration">
                      <Latency ms={d?.execTime} />
                    </DetailRow>
                    <DetailRow label="Received" hint={created ? created.toLocaleString() : undefined}>
                      <TimeAgo value={entry.created} />
                    </DetailRow>
                  </DetailList>
                </DetailSection>

                <DetailSection title="Client">
                  <DetailList>
                    <DetailRow label="Caller">
                      <span className="font-mono text-xs">{callerLabel(entry)}</span>
                    </DetailRow>
                    <DetailRow label="Client IP">
                      <span className="font-mono text-xs">{d?.userIP || d?.remoteIP || "—"}</span>
                    </DetailRow>
                    {d?.remoteIP && d.userIP && d.remoteIP !== d.userIP ? (
                      <DetailRow label="Remote IP">
                        <span className="font-mono text-xs">{d.remoteIP}</span>
                      </DetailRow>
                    ) : null}
                    <DetailRow label="User agent">
                      <span className="text-xs">{d?.userAgent || "—"}</span>
                    </DetailRow>
                  </DetailList>
                </DetailSection>

                <DetailSection title="Log entry" defaultOpen={false}>
                  <DetailList>
                    <DetailRow label="Log id">
                      <span className="font-mono text-xs">{entry.id}</span>
                    </DetailRow>
                    <DetailRow label="Level">
                      <span className="font-tabular">{entry.level}</span>
                    </DetailRow>
                    <DetailRow label="Message">
                      <span className="font-mono text-xs break-all">{entry.message}</span>
                    </DetailRow>
                  </DetailList>
                </DetailSection>
              </DetailBody>
            </DetailTabPanel>
            <DetailTabPanel value="raw">
              <DetailBody>
                <CodeBlock title="Raw entry" json={json} copyLabel="Copy JSON" />
              </DetailBody>
            </DetailTabPanel>
          </DetailTabs>
        </>
      ) : null}
    </DetailSheet>
  );
}

/** The slice of settings this page owns — moved here from Application so
 * retention lives next to the log table it governs. */
type LogsDraft = Pick<ServerSettings, "logs">;

function logsDraftOf(settings: ServerSettings): LogsDraft {
  return { logs: { ...settings.logs } };
}

function logsDirtyAgainst(draft: LogsDraft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(logsDraftOf(settings));
}

/** Superuser-only view over `_request_logs`, the bounded history the
 * `request_log` middleware writes on every `/api/*` call. Read-only table
 * with a live tail, quick filters and a detail drawer; retention settings
 * sit underneath. */
export function RequestLogsPage() {
  const urlSearch = settingsLogsRoute.useSearch();
  const navigate = settingsLogsRoute.useNavigate();
  const page = urlSearch.page ?? 1;
  const filter = urlSearch.filter ?? "";
  const [filterInput, setFilterInput] = useState(filter);
  const [method, setMethod] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [live, setLive] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const { data: settings } = useSettings();
  const saveLogs = useSettingsMutation();
  const [logsDraft, setLogsDraft] = useState<LogsDraft | null>(null);
  const [logsSeedKey, setLogsSeedKey] = useState<ServerSettings | undefined>(settings);

  // Adopt server state on first load and on any refetch that lands while
  // nothing is being edited; never clobber an edit in progress.
  if (settings && (logsSeedKey !== settings || logsDraft === null)) {
    setLogsSeedKey(settings);
    setLogsDraft(logsDraftOf(settings));
  }

  const logsErrors = useMemo(() => {
    if (!logsDraft) return [];
    const errors: string[] = [];
    if (logsDraft.logs.maxDays < 0) errors.push("Log retention can't be negative");
    if (logsDraft.logs.maxDataSize < 0) errors.push("Max data size can't be negative");
    if (logsDraft.logs.mailLogMaxDays < 0) errors.push("Mail log retention can't be negative");
    return errors;
  }, [logsDraft]);

  function patchLogs(next: Partial<ServerSettings["logs"]>) {
    setLogsDraft((d) => (d ? { logs: { ...d.logs, ...next } } : d));
  }
  const logsDirty = logsDraft && settings ? logsDirtyAgainst(logsDraft, settings) : false;

  function submitLogs() {
    if (!logsDraft || logsErrors.length > 0) return;
    saveLogs.mutate(logsDraft, {
      onSuccess: () => toast.success("Request log settings saved"),
      onError: (error) => {
        const failed = describeFailure(error);
        toast.error(failed.title, { description: failed.detail || failed.serverMessage || undefined });
      },
    });
  }
  useSaveShortcut({
    dirty: logsDirty,
    canSave: logsErrors.length === 0 && !saveLogs.isPending,
    onSave: submitLogs,
  });

  // Typing in the search box updates the URL filter after a short pause —
  // each keystroke would otherwise hit the server with a half-typed query.
  useEffect(() => {
    setFilterInput(filter);
  }, [filter]);
  useEffect(() => {
    if (filterInput.trim() === filter) return;
    const timer = window.setTimeout(() => {
      void navigate({
        search: (prev) => ({ tab: prev.tab, page: undefined, filter: filterInput.trim() || undefined }),
        replace: true,
      });
    }, 350);
    return () => window.clearTimeout(timer);
  }, [filterInput, filter, navigate]);

  const effectiveFilter = useMemo(
    () =>
      [textExpression(filter), method === "all" ? null : `data.method = "${method}"`, STATUS_EXPR[statusFilter]]
        .filter(Boolean)
        .join(" && "),
    [filter, method, statusFilter],
  );
  const filtering = effectiveFilter !== "";

  const { data, isPending, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["request-logs", page, effectiveFilter],
    queryFn: () =>
      cb.send<RequestLogsResult>("/api/logs", {
        method: "GET",
        query: { page, perPage: 30, filter: effectiveFilter || undefined },
      }),
    placeholderData: (previous) => previous,
    // Live tail only makes sense on the newest page.
    refetchInterval: live && page === 1 ? LIVE_INTERVAL : false,
  });
  const { data: stats } = useHourlyStats(effectiveFilter, live);

  const items = useMemo(() => data?.items ?? [], [data]);

  const selectedIndex = selectedId ? items.findIndex((e) => e.id === selectedId) : -1;
  const selected = selectedIndex >= 0 ? items[selectedIndex] : null;
  function step(delta: -1 | 1) {
    const next = items[selectedIndex + delta];
    if (next) setSelectedId(next.id);
  }

  const total24h = useMemo(() => (stats ?? []).reduce((sum, b) => sum + b.total, 0), [stats]);
  const errorsInView = items.filter((e) => (e.data?.status ?? 0) >= 400).length;
  const timed = items.filter((e) => e.data?.execTime != null);
  const avgMs = timed.length ? timed.reduce((sum, e) => sum + (e.data!.execTime ?? 0), 0) / timed.length : null;

  function clearFilters() {
    setFilterInput("");
    setMethod("all");
    setStatusFilter("all");
    void navigate({ search: (prev) => ({ tab: prev.tab }), replace: true });
  }

  const silenced = (settings?.logs.minLevel ?? 0) >= 4;

  return (
    <SettingsPage
      title={ITEM.label}
      description={ITEM.description}
      width="wide"
      action={
        <>
          <Button
            variant="outline"
            size="sm"
            aria-pressed={live}
            onClick={() => setLive((v) => !v)}
            title={live ? "Stop following new requests" : "Follow new requests as they arrive"}
          >
            <StatusDot tone={live ? "success" : "neutral"} pulse={live} />
            {live ? "Live" : "Go live"}
          </Button>
          <Button
            variant="outline"
            size="icon-sm"
            aria-label="Refresh logs"
            title="Refresh"
            onClick={() => void refetch()}
          >
            <RefreshCw className={cn(isFetching && "motion-safe:animate-spin")} />
          </Button>
        </>
      }
    >
      <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
        <StatCard
          index={0}
          label="Requests, last 24 h"
          value={total24h.toLocaleString()}
          loading={!stats}
          hint={filtering ? "matching current filter" : undefined}
        />
        <StatCard
          index={1}
          label="Matching rows"
          value={(data?.totalItems ?? 0).toLocaleString()}
          loading={isPending}
          hint={`${items.length} on this page`}
        />
        <StatCard
          index={2}
          label="Errors on this page"
          value={errorsInView}
          loading={isPending}
          tone={errorsInView > 0 ? "danger" : "neutral"}
          hint="status 400 and above"
        />
        <StatCard
          index={3}
          label="Avg latency on this page"
          value={avgMs == null ? "—" : formatMs(avgMs)}
          loading={isPending}
          hint={`slow ≥ ${SLOW_MS} ms`}
        />
      </div>

      <RequestLogsChart stats={stats} />

      <div className="sticky top-0 z-sticky -mx-1 bg-background/95 px-1 py-1 backdrop-blur-sm">
        <DataTableToolbar
          search={{
            value: filterInput,
            onChange: setFilterInput,
            placeholder: "Search path, or filter: data.status = 404",
            label: "Filter request logs",
          }}
          filters={
            <>
              <Select value={method} onValueChange={setMethod}>
                <SelectTrigger size="sm" aria-label="Filter by method" className="h-control-sm w-32">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All methods</SelectItem>
                  {METHODS.map((m) => (
                    <SelectItem key={m} value={m}>
                      {m}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as StatusFilter)}>
                <SelectTrigger size="sm" aria-label="Filter by outcome" className="h-control-sm w-44">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {STATUS_FILTERS.map((f) => (
                    <SelectItem key={f.value} value={f.value}>
                      {f.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {filtering ? (
                <Button variant="ghost" size="sm" onClick={clearFilters}>
                  Clear filters
                </Button>
              ) : null}
            </>
          }
        />
      </div>

      {isPending ? (
        <ListSkeleton rows={8} />
      ) : isError ? (
        <ErrorState
          title="Couldn't load request logs"
          error={error}
          onRetry={() => void refetch()}
          compact
        />
      ) : items.length === 0 ? (
        filtering ? (
          <EmptyState
            compact
            icon={<ListTree />}
            title="No requests match"
            description="Nothing in the log fits these filters. Loosen the search or clear the filters."
            action={
              <Button variant="outline" size="sm" onClick={clearFilters}>
                Clear filters
              </Button>
            }
          />
        ) : (
          <EmptyState
            compact
            icon={<ListTree />}
            title="No requests logged yet"
            description={
              silenced
                ? "Minimum level is set above Info, which silences request rows. Lower it under Retention below to record requests."
                : "Every call to /api/* lands here as it happens. Make a request, or switch on Live to watch for one."
            }
            action={
              <Button variant="outline" size="sm" onClick={() => setLive(true)} disabled={live}>
                <StatusDot tone="neutral" />
                Go live
              </Button>
            }
          />
        )
      ) : (
        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-40">Time</TableHead>
                <TableHead className="w-20">Method</TableHead>
                <TableHead>Path</TableHead>
                <TableHead className="w-24">Status</TableHead>
                <TableHead className="w-36">Duration</TableHead>
                <TableHead className="hidden w-40 lg:table-cell">Caller</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((entry) => {
                const status = entry.data?.status;
                const created = timeOf(entry);
                return (
                  <TableRow
                    key={entry.id}
                    tabIndex={0}
                    role="button"
                    aria-label={`${entry.data?.method ?? ""} ${pathOf(entry)} — ${status ?? "no status"}, open details`}
                    data-state={entry.id === selectedId ? "selected" : undefined}
                    onClick={() => setSelectedId(entry.id)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        setSelectedId(entry.id);
                      }
                    }}
                    className="cursor-pointer"
                  >
                    <TableCell className="font-tabular text-xs whitespace-nowrap text-muted-foreground" title={created.toLocaleString()}>
                      {created.toLocaleTimeString([], { hour12: false })}
                      <span className="ml-1.5 text-muted-foreground/70">
                        {created.toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                      </span>
                    </TableCell>
                    <TableCell className="font-mono text-xs font-medium">{entry.data?.method ?? "—"}</TableCell>
                    <TableCell className="max-w-0 truncate font-mono text-xs" title={pathOf(entry)}>
                      {pathOf(entry)}
                    </TableCell>
                    <TableCell>
                      {status != null ? <StatusBadge tone={statusTone(status)}>{status}</StatusBadge> : "—"}
                    </TableCell>
                    <TableCell>
                      <Latency ms={entry.data?.execTime} />
                    </TableCell>
                    <TableCell className="hidden max-w-0 truncate text-xs text-muted-foreground lg:table-cell">
                      {callerLabel(entry)}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      {data && data.totalPages > 1 ? (
        <div className="flex items-center justify-between gap-3 py-1">
          <span className="font-tabular text-xs text-muted-foreground">
            {data.totalItems.toLocaleString()} requests · page {page} of {data.totalPages}
          </span>
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

      <RequestDetailSheet
        entry={selected}
        index={selectedIndex}
        total={items.length}
        onClose={() => setSelectedId(null)}
        onStep={step}
      />

      {logsDraft ? (
        <SettingsSection title="Retention" description="What the server records about incoming requests, and for how long.">
          <SettingRow label="Retention" htmlFor="logs-days" help="0 keeps logs forever. Older rows are pruned.">
            <NumberSetting
              id="logs-days"
              min={0}
              value={logsDraft.logs.maxDays}
              onChange={(maxDays) => patchLogs({ maxDays })}
              suffix="days"
              invalid={logsDraft.logs.maxDays < 0}
            />
          </SettingRow>
          <SettingRow
            label="Minimum level"
            htmlFor="logs-min-level"
            help={LOG_LEVELS.find((l) => l.value === logsDraft.logs.minLevel)?.help}
          >
            <Select
              value={String(logsDraft.logs.minLevel)}
              onValueChange={(next) => patchLogs({ minLevel: Number(next) })}
            >
              <SelectTrigger id="logs-min-level" className="h-control-md w-56">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LOG_LEVELS.map((l) => (
                  <SelectItem key={l.value} value={String(l.value)}>
                    {l.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </SettingRow>
          <SettingRow label="Record client IP" htmlFor="logs-ip" help="Turn off if IP addresses count as personal data for you.">
            <ToggleSetting
              id="logs-ip"
              checked={logsDraft.logs.logIP}
              onChange={(logIP) => patchLogs({ logIP })}
              label={logsDraft.logs.logIP ? "Stored with each request" : "Not stored"}
            />
          </SettingRow>
          <SettingRow
            label="Record caller id"
            htmlFor="logs-auth"
            help="Stores which authenticated record made the request, so a log line can be traced to a user."
          >
            <ToggleSetting
              id="logs-auth"
              checked={logsDraft.logs.logAuthId}
              onChange={(logAuthId) => patchLogs({ logAuthId })}
              label={logsDraft.logs.logAuthId ? "Stored with each request" : "Not stored"}
            />
          </SettingRow>
          <SettingRow
            label="Max data size"
            htmlFor="logs-max-data"
            help="Truncate a log entry's recorded request/response data past this many bytes. 0 means no limit."
          >
            <NumberSetting
              id="logs-max-data"
              min={0}
              value={logsDraft.logs.maxDataSize}
              onChange={(maxDataSize) => patchLogs({ maxDataSize })}
              suffix="bytes"
              invalid={logsDraft.logs.maxDataSize < 0}
            />
          </SettingRow>
          <SettingRow
            label="Mail log retention"
            htmlFor="logs-mail-retention"
            help="How long _mailLog rows (every POST /api/mails/send / _emailTriggers send attempt) are kept, pruned on the same cadence as these request logs. 0 keeps them forever."
          >
            <NumberSetting
              id="logs-mail-retention"
              min={0}
              value={logsDraft.logs.mailLogMaxDays}
              onChange={(mailLogMaxDays) => patchLogs({ mailLogMaxDays })}
              suffix="days"
              invalid={logsDraft.logs.mailLogMaxDays < 0}
            />
          </SettingRow>
        </SettingsSection>
      ) : null}
      {logsDraft ? (
        <SettingsSaveBar
          dirty={logsDirty}
          pending={saveLogs.isPending}
          errors={logsErrors}
          onSave={submitLogs}
          onReset={() => settings && setLogsDraft(logsDraftOf(settings))}
        />
      ) : null}
    </SettingsPage>
  );
}
