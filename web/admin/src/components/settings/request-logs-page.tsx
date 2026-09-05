import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Bar, BarChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsLogsRoute } from "@/routes/settings-logs";
import { settingsItemFor } from "@/lib/settings-nav";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Pagination } from "@/components/ui/pagination";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import {
  NumberSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  ToggleSetting,
} from "@/components/settings/settings-form";
import { ListTree } from "lucide-react";

/** `GET /api/logs/stats` — hourly request counts for whatever filter the
 * table itself is showing, so the chart and the rows underneath it always
 * agree on what "the current view" means. */
type LogStat = { date: string; total: number };

/** Renders as a plain hour for a bucket less than a day old, and adds the
 * date once the range spans more than one — the hourly label alone stops
 * being enough to tell two bars apart once they're a day apart. */
function bucketLabel(iso: string, spansMultipleDays: boolean): string {
  const date = new Date(iso);
  const hour = date.toLocaleTimeString(undefined, { hour: "numeric" });
  if (!spansMultipleDays) return hour;
  return `${date.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${hour}`;
}

/** The activity chart above the log table — hourly request volume for the
 * current filter. Superuser-only like the rest of this screen; there's
 * nothing here that isn't already visible row-by-row in the table, this
 * is just the shape of it at a glance. */
function RequestLogsChart({ filter }: { filter: string }) {
  const { data: stats } = useQuery({
    queryKey: ["request-logs", "stats", filter],
    queryFn: () => cb.send<LogStat[]>("/api/logs/stats", { method: "GET", query: { filter: filter || undefined } }),
    placeholderData: (previous) => previous,
  });

  const spansMultipleDays = useMemo(() => {
    if (!stats || stats.length < 2) return false;
    const first = new Date(stats[0]!.date).toDateString();
    const last = new Date(stats[stats.length - 1]!.date).toDateString();
    return first !== last;
  }, [stats]);

  const chartData = useMemo(
    () => (stats ?? []).map((s) => ({ ...s, label: bucketLabel(s.date, spansMultipleDays) })),
    [stats, spansMultipleDays],
  );

  if (!stats || stats.length === 0) return null;

  return (
    <div className="h-32 w-full rounded-lg border border-border bg-card p-2">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={chartData} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
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
          <Bar dataKey="total" radius={[2, 2, 0, 0]} className="fill-primary" />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

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
 * `request_log` middleware writes on every `/api/*` call. Read-only —
 * there's nothing to edit here, just something to search. */
export function RequestLogsPage() {
  const urlSearch = settingsLogsRoute.useSearch();
  const navigate = settingsLogsRoute.useNavigate();
  const page = urlSearch.page ?? 1;
  const filter = urlSearch.filter ?? "";
  const [filterInput, setFilterInput] = useState(filter);

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

  const item = settingsItemFor("/settings/logs")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <RequestLogsChart filter={filter} />

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

      {logsDraft ? (
        <SettingsSection title="Retention" description="What the server records about incoming requests.">
          <SettingRow label="Retention" htmlFor="logs-days" help="0 keeps logs forever. Older rows are pruned.">
            <NumberSetting
              id="logs-days"
              min={0}
              value={logsDraft.logs.maxDays}
              onChange={(maxDays) => patchLogs({ maxDays })}
              suffix="days"
            />
          </SettingRow>
          <SettingRow label="Record client IP" htmlFor="logs-ip">
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
