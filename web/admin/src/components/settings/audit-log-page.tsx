import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, Copy, History } from "lucide-react";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { settingsLogsRoute } from "@/routes/settings-logs";
import { settingsItemFor } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatusBadge, type Tone } from "@/components/layout/stat-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Pagination } from "@/components/ui/pagination";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { SettingsPage } from "@/components/settings/settings-form";
import { LIVE_INTERVAL_MS, LiveToggle } from "@/components/settings/live-toggle";
import { TimeAgo } from "@/components/settings/time-ago";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** Every action `crates/server/src/audit.rs` writes. Kept in sync with that
 * module's `write(...)` call sites by hand — there is no server endpoint
 * enumerating them, and the list is short and stable. */
const ACTIONS = [
  "collection.create",
  "collection.update",
  "collection.delete",
  "record.delete",
  "superuser.create",
  "superuser.role_change",
  "superuser.delete",
  "settings.update",
] as const;

const PER_PAGE = 30;
const HEAD = "h-control-md text-xs font-medium text-muted-foreground";
const CELL = "py-1.5";

/** A `_audit_log` record, expanded with the acting superuser's email so
 * the table never has to show a bare relation id. */
type AuditLogEntry = {
  id: string;
  actor: string;
  action: string;
  target: string;
  meta: Record<string, unknown>;
  created: string;
  expand?: {
    actor?: { email?: string };
  };
};

type AuditLogResult = {
  page: number;
  perPage: number;
  totalItems: number;
  totalPages: number;
  items: AuditLogEntry[];
};

/** Deletions are the one category of audit row an operator scans for
 * first; everything else is routine change history. */
function actionTone(action: string): Tone {
  if (action.endsWith(".delete")) return "danger";
  if (action.endsWith(".update") || action.endsWith(".role_change")) return "warning";
  if (action.endsWith(".create")) return "success";
  return "neutral";
}

/** `_audit_log.actor` is blanked to an empty string server-side when the
 * referenced superuser is later deleted (`crates/server/src/audit.rs`) —
 * there is no "system"-initiated audit write today, so an empty actor
 * means a since-deleted superuser, not some automated process. A non-empty
 * `actor` id whose `expand` didn't come back is a different, rarer case:
 * the record genuinely couldn't be resolved. */
function actorLabel(entry: AuditLogEntry): string {
  if (entry.expand?.actor?.email) return entry.expand.actor.email;
  return entry.actor ? "unknown" : "removed superuser";
}

/** Superuser-only view over `_audit_log`, the append-only history
 * `crates/server/src/audit.rs` writes for every consequential
 * dashboard/API action. Read-only — the collection itself rejects every
 * update and delete, even from a superuser, so there is nothing to edit
 * here, just something to search. */
export function AuditLogPage() {
  const urlSearch = settingsLogsRoute.useSearch();
  const navigate = settingsLogsRoute.useNavigate();
  const page = urlSearch.page ?? 1;
  const action = urlSearch.action ?? "";
  const from = urlSearch.from ?? "";
  const to = urlSearch.to ?? "";
  const [selected, setSelected] = useState<AuditLogEntry | null>(null);
  const [live, setLive] = useState(true);

  const filterParts: string[] = [];
  if (action) filterParts.push(`action = "${action}"`);
  if (from) filterParts.push(`created >= "${from} 00:00:00"`);
  if (to) filterParts.push(`created <= "${to} 23:59:59"`);
  const filter = filterParts.join(" && ");
  const hasFilters = filter !== "";

  // New entries land on page 1; polling a deeper page or while the detail
  // sheet is open would shuffle rows under the reader, so it pauses there.
  const refetchInterval = live && page === 1 && selected === null ? LIVE_INTERVAL_MS : false;

  const { data, isPending, error, refetch, isFetching } = useQuery({
    queryKey: ["audit-log", page, action, from, to],
    queryFn: () =>
      cb.collection("_audit_log").list({
        page,
        perPage: PER_PAGE,
        filter: filter || undefined,
        sort: "-created",
        expand: "actor",
      }) as unknown as Promise<AuditLogResult>,
    placeholderData: (previous) => previous,
    refetchInterval,
  });

  function updateSearch(patch: AuditLogSearchPatch) {
    void navigate({
      search: (prev) => ({ ...prev, page: undefined, ...patch }),
      replace: true,
    });
  }

  const items = data?.items ?? [];
  const firstRow = data ? (data.page - 1) * data.perPage + 1 : 0;

  const item = settingsItemFor("/settings/audit")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <DataTableToolbar
        filters={
          <>
            <Select
              value={action || "all"}
              onValueChange={(next) => updateSearch({ action: next === "all" ? undefined : next })}
            >
              <SelectTrigger id="audit-action-filter" aria-label="Action" className="h-control-sm w-52 font-mono text-xs">
                <SelectValue placeholder="All actions" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All actions</SelectItem>
                {ACTIONS.map((a) => (
                  <SelectItem key={a} value={a} className="font-mono text-xs">
                    {a}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="flex items-center gap-1.5">
              <Label htmlFor="audit-from" className="text-xs text-muted-foreground">
                From
              </Label>
              <Input
                id="audit-from"
                type="date"
                value={from}
                max={to || undefined}
                onChange={(e) => updateSearch({ from: e.target.value || undefined })}
                className="h-control-sm w-36 font-mono text-xs"
              />
            </div>
            <div className="flex items-center gap-1.5">
              <Label htmlFor="audit-to" className="text-xs text-muted-foreground">
                To
              </Label>
              <Input
                id="audit-to"
                type="date"
                value={to}
                min={from || undefined}
                onChange={(e) => updateSearch({ to: e.target.value || undefined })}
                className="h-control-sm w-36 font-mono text-xs"
              />
            </div>
            {hasFilters ? (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => updateSearch({ action: undefined, from: undefined, to: undefined })}
              >
                Clear filters
              </Button>
            ) : null}
          </>
        }
        actions={
          <LiveToggle live={live} onLiveChange={setLive} fetching={isFetching} onRefresh={() => void refetch()} />
        }
      />

      {error && !data ? (
        <ErrorState compact title="Couldn't load the audit log" error={describeFailure(error).detail} onRetry={() => void refetch()} />
      ) : isPending ? (
        <ListSkeleton rows={8} />
      ) : items.length === 0 ? (
        <EmptyState
          compact
          icon={<History />}
          title={hasFilters ? "No entries match these filters" : "No audited actions yet"}
          description={
            hasFilters
              ? "Widen the date range or pick a different action."
              : "Schema changes, settings edits, superuser account changes and superuser-bypassed record deletions will show up here as they happen."
          }
          action={
            hasFilters ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => updateSearch({ action: undefined, from: undefined, to: undefined })}
              >
                Clear filters
              </Button>
            ) : undefined
          }
        />
      ) : (
        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className={cn(HEAD, "pl-3")}>Time</TableHead>
                <TableHead className={HEAD}>Actor</TableHead>
                <TableHead className={HEAD}>Action</TableHead>
                <TableHead className={HEAD}>Target</TableHead>
                <TableHead className={cn(HEAD, "w-20 pr-3")}>
                  <span className="sr-only">Details</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((entry) => {
                const hasMeta = Object.keys(entry.meta ?? {}).length > 0;
                return (
                  <TableRow key={entry.id}>
                    <TableCell className={cn(CELL, "pl-3 text-xs")}>
                      <span className="flex items-baseline gap-2">
                        <span className="font-mono font-tabular">{parseServerDate(entry.created).toLocaleString()}</span>
                        <TimeAgo value={entry.created} className="text-2xs text-muted-foreground" />
                      </span>
                    </TableCell>
                    <TableCell
                      className={cn(
                        CELL,
                        "max-w-56 truncate text-xs",
                        !entry.expand?.actor?.email && "text-muted-foreground italic",
                      )}
                    >
                      {actorLabel(entry)}
                    </TableCell>
                    <TableCell className={CELL}>
                      <StatusBadge tone={actionTone(entry.action)} className="font-mono">
                        {entry.action}
                      </StatusBadge>
                    </TableCell>
                    <TableCell className={cn(CELL, "max-w-64 truncate font-mono text-xs")} title={entry.target}>
                      {entry.target || <span className="text-muted-foreground">—</span>}
                    </TableCell>
                    <TableCell className={cn(CELL, "pr-3 text-right")}>
                      <Button
                        variant="ghost"
                        size="xs"
                        aria-label={`Details for ${entry.action} on ${entry.target || "—"}`}
                        onClick={() => setSelected(entry)}
                      >
                        {hasMeta ? "Details" : "View"}
                      </Button>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      {data && data.totalItems > 0 ? (
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="font-tabular text-xs text-muted-foreground">
            {firstRow}–{firstRow + items.length - 1} of {data.totalItems}
          </p>
          {data.totalPages > 1 ? (
            <Pagination
              count={data.totalPages}
              page={page}
              onPageChange={(next) =>
                void navigate({ search: (prev) => ({ ...prev, page: next > 1 ? next : undefined }), replace: true })
              }
              label="Audit log pages"
            />
          ) : null}
        </div>
      ) : null}

      <EntrySheet entry={selected} onClose={() => setSelected(null)} />
    </SettingsPage>
  );
}

/** Everything about one audit row, with the raw `meta` JSON copyable. */
function EntrySheet({ entry, onClose }: { entry: AuditLogEntry | null; onClose: () => void }) {
  const { copy, status } = useCopyToClipboard();
  const meta = entry ? JSON.stringify(entry.meta ?? {}, null, 2) : "";
  const hasMeta = entry ? Object.keys(entry.meta ?? {}).length > 0 : false;

  return (
    <Sheet open={entry !== null} onOpenChange={(open) => !open && onClose()}>
      <SheetContent side="right" className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-lg">
        {entry ? (
          <>
            <SheetHeader>
              <SheetTitle className="flex flex-wrap items-center gap-2">
                <StatusBadge tone={actionTone(entry.action)} className="font-mono">
                  {entry.action}
                </StatusBadge>
              </SheetTitle>
              <SheetDescription>
                <TimeAgo value={entry.created} /> · <span className="font-mono">{parseServerDate(entry.created).toLocaleString()}</span>
              </SheetDescription>
            </SheetHeader>
            <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4">
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
                <dt className="text-muted-foreground">Actor</dt>
                <dd className="min-w-0 break-words">{actorLabel(entry)}</dd>
                <dt className="text-muted-foreground">Target</dt>
                <dd className="min-w-0 font-mono text-xs break-all">{entry.target || "—"}</dd>
                <dt className="text-muted-foreground">Entry id</dt>
                <dd className="min-w-0 font-mono text-xs break-all">{entry.id}</dd>
              </dl>

              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-xs font-medium">Details</span>
                  {hasMeta ? (
                    <Button variant="ghost" size="xs" onClick={() => void copy(meta)}>
                      {status === "copied" ? <Check className="text-success" /> : <Copy />}
                      {status === "copied" ? "Copied" : "Copy JSON"}
                    </Button>
                  ) : null}
                </div>
                {hasMeta ? (
                  <pre className="overflow-x-auto rounded-lg border border-border bg-surface-sunken p-3 font-mono text-xs leading-relaxed">
                    {meta}
                  </pre>
                ) : (
                  <p className="text-sm text-muted-foreground">This action recorded no extra details.</p>
                )}
              </div>
            </div>
          </>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

type AuditLogSearchPatch = {
  action?: string;
  from?: string;
  to?: string;
};
