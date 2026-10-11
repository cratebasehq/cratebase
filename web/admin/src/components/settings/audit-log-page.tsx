import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Copy, History } from "lucide-react";
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
import {
  DetailBody,
  DetailEmpty,
  DetailList,
  DetailRow,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
  DetailTabPanel,
  DetailTabs,
} from "@/components/layout/detail-sheet";
import { CodeBlock } from "@/components/settings/detail-json";
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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [live, setLive] = useState(true);

  const filterParts: string[] = [];
  if (action) filterParts.push(`action = "${action}"`);
  if (from) filterParts.push(`created >= "${from} 00:00:00"`);
  if (to) filterParts.push(`created <= "${to} 23:59:59"`);
  const filter = filterParts.join(" && ");
  const hasFilters = filter !== "";

  // New entries land on page 1; polling a deeper page or while the detail
  // sheet is open would shuffle rows under the reader, so it pauses there.
  const refetchInterval = live && page === 1 && selectedId === null ? LIVE_INTERVAL_MS : false;

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
  const selectedIndex = selectedId ? items.findIndex((e) => e.id === selectedId) : -1;
  const selected = selectedIndex >= 0 ? items[selectedIndex] : null;
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
                        onClick={() => setSelectedId(entry.id)}
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

      <EntrySheet
        entry={selected}
        index={selectedIndex}
        total={items.length}
        onClose={() => setSelectedId(null)}
        onStep={(delta) => {
          const next = items[selectedIndex + delta];
          if (next) setSelectedId(next.id);
        }}
      />
    </SettingsPage>
  );
}

/** Everything about one audit row, with the raw `meta` JSON copyable. */
function EntrySheet({
  entry,
  index,
  total,
  onClose,
  onStep,
}: {
  entry: AuditLogEntry | null;
  index: number;
  total: number;
  onClose: () => void;
  onStep: (delta: -1 | 1) => void;
}) {
  const { copy } = useCopyToClipboard();
  const [tab, setTab] = useState("overview");
  const meta = entry ? JSON.stringify(entry.meta ?? {}, null, 2) : "";
  const raw = entry ? JSON.stringify(entry, null, 2) : "";
  const metaCount = entry ? Object.keys(entry.meta ?? {}).length : 0;

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
            menu={[{ label: "Copy JSON", onSelect: () => void copy(raw), icon: <Copy /> }]}
            title={<span className="font-mono text-base">{entry.action}</span>}
            description={`Audit entry ${entry.action}`}
            status={
              <StatusBadge tone={actionTone(entry.action)}>{entry.action.split(".").pop() ?? entry.action}</StatusBadge>
            }
            meta={[actorLabel(entry), <TimeAgo key="a" value={entry.created} />]}
            actions={
              <Button size="sm" variant="outline" onClick={() => void copy(raw)}>
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
                <DetailSection title="Entry">
                  <DetailList>
                    <DetailRow label="Actor">
                      <span className={cn(!entry.expand?.actor?.email && "text-muted-foreground italic")}>
                        {actorLabel(entry)}
                      </span>
                    </DetailRow>
                    <DetailRow label="Action">
                      <span className="font-mono text-xs">{entry.action}</span>
                    </DetailRow>
                    <DetailRow label="Target">
                      <span className="font-mono text-xs break-all">{entry.target || "—"}</span>
                    </DetailRow>
                    <DetailRow label="When" hint={parseServerDate(entry.created).toLocaleString()}>
                      <TimeAgo value={entry.created} />
                    </DetailRow>
                    <DetailRow label="Entry id">
                      <span className="font-mono text-xs break-all">{entry.id}</span>
                    </DetailRow>
                  </DetailList>
                </DetailSection>
                <DetailSection title="Details">
                  {metaCount > 0 ? (
                    <CodeBlock json={meta} copyLabel="Copy JSON" />
                  ) : (
                    <DetailEmpty title="No extra details" description="This action recorded no extra details." />
                  )}
                </DetailSection>
              </DetailBody>
            </DetailTabPanel>
            <DetailTabPanel value="raw">
              <DetailBody>
                <CodeBlock title="Raw entry" json={raw} copyLabel="Copy JSON" />
              </DetailBody>
            </DetailTabPanel>
          </DetailTabs>
        </>
      ) : null}
    </DetailSheet>
  );
}

type AuditLogSearchPatch = {
  action?: string;
  from?: string;
  to?: string;
};
