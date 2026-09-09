import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { History, ShieldAlert } from "lucide-react";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { settingsAuditRoute } from "@/routes/settings-audit";
import { settingsItemFor } from "@/lib/settings-nav";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Pagination } from "@/components/ui/pagination";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { SettingsPage } from "@/components/settings/settings-form";
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
function actionVariant(action: string): "default" | "secondary" | "destructive" {
  if (action.endsWith(".delete")) return "destructive";
  if (action.endsWith(".update") || action.endsWith(".role_change")) return "secondary";
  return "default";
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
  const urlSearch = settingsAuditRoute.useSearch();
  const navigate = settingsAuditRoute.useNavigate();
  const page = urlSearch.page ?? 1;
  const action = urlSearch.action ?? "";
  const from = urlSearch.from ?? "";
  const to = urlSearch.to ?? "";
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  const filterParts: string[] = [];
  if (action) filterParts.push(`action = "${action}"`);
  if (from) filterParts.push(`created >= "${from} 00:00:00"`);
  if (to) filterParts.push(`created <= "${to} 23:59:59"`);
  const filter = filterParts.join(" && ");

  const { data, isLoading, error } = useQuery({
    queryKey: ["audit-log", page, action, from, to],
    queryFn: () =>
      cb.collection("_audit_log").list({
        page,
        perPage: 30,
        filter: filter || undefined,
        sort: "-created",
        expand: "actor",
      }) as unknown as Promise<AuditLogResult>,
    placeholderData: (previous) => previous,
  });

  function updateSearch(patch: Partial<AuditLogSearchPatch>) {
    void navigate({
      search: (prev) => ({ ...prev, page: undefined, ...patch }),
      replace: true,
    });
  }

  const item = settingsItemFor("/settings/audit")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">

      <div className="flex flex-wrap items-end gap-2">
        <div className="flex flex-col gap-1">
          <label htmlFor="audit-action-filter" className="text-xs text-muted-foreground">
            Action
          </label>
          <Select
            value={action || "all"}
            onValueChange={(next) => updateSearch({ action: next === "all" ? undefined : next })}
          >
            <SelectTrigger id="audit-action-filter" className="w-52">
              <SelectValue placeholder="All actions" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All actions</SelectItem>
              {ACTIONS.map((a) => (
                <SelectItem key={a} value={a}>
                  {a}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor="audit-from" className="text-xs text-muted-foreground">
            From
          </label>
          <Input
            id="audit-from"
            type="date"
            value={from}
            onChange={(e) => updateSearch({ from: e.target.value || undefined })}
            className="w-40"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor="audit-to" className="text-xs text-muted-foreground">
            To
          </label>
          <Input
            id="audit-to"
            type="date"
            value={to}
            onChange={(e) => updateSearch({ to: e.target.value || undefined })}
            className="w-40"
          />
        </div>
      </div>

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ShieldAlert />
            </EmptyMedia>
            <EmptyTitle>Couldn't load the audit log</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[200px]">Actor</TableHead>
              <TableHead className="w-[170px]">Action</TableHead>
              <TableHead>Target</TableHead>
              <TableHead className="w-[180px]">Time</TableHead>
              <TableHead className="w-[80px]" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {(data?.items ?? []).map((entry) => {
              const hasMeta = entry.meta && Object.keys(entry.meta).length > 0;
              const isOpen = expanded[entry.id] ?? false;
              return (
                <Collapsible key={entry.id} asChild open={isOpen} onOpenChange={(open) => setExpanded((prev) => ({ ...prev, [entry.id]: open }))}>
                  <>
                    <TableRow>
                      <TableCell className="text-xs">{actorLabel(entry)}</TableCell>
                      <TableCell>
                        <Badge variant={actionVariant(entry.action)} className="font-mono text-[11px]">
                          {entry.action}
                        </Badge>
                      </TableCell>
                      <TableCell className="font-mono text-xs">{entry.target}</TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {parseServerDate(entry.created).toLocaleString()}
                      </TableCell>
                      <TableCell className="text-right">
                        {hasMeta ? (
                          <CollapsibleTrigger asChild>
                            <button
                              type="button"
                              className="text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                            >
                              {isOpen ? "Hide" : "Details"}
                            </button>
                          </CollapsibleTrigger>
                        ) : null}
                      </TableCell>
                    </TableRow>
                    {hasMeta ? (
                      <TableRow>
                        <TableCell colSpan={5} className="p-0">
                          <CollapsibleContent>
                            <pre className="overflow-x-auto bg-muted/40 p-3 text-[11px] leading-relaxed">
                              {JSON.stringify(entry.meta, null, 2)}
                            </pre>
                          </CollapsibleContent>
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </>
                </Collapsible>
              );
            })}
            {isLoading ? (
              Array.from({ length: 5 }).map((_, i) => (
                <TableRow key={i}>
                  <TableCell colSpan={5}>
                    <Skeleton className="h-row w-full" />
                  </TableCell>
                </TableRow>
              ))
            ) : null}
            {!isLoading && !error && (data?.items.length ?? 0) === 0 ? (
              <TableRow>
                <TableCell colSpan={5} className="py-8">
                  <Empty>
                    <EmptyHeader>
                      <EmptyMedia variant="icon">
                        <History />
                      </EmptyMedia>
                      <EmptyTitle>No audited actions yet</EmptyTitle>
                      <EmptyDescription>
                        Schema changes, settings edits, superuser account changes and superuser-bypassed record
                        deletions will show up here as they happen.
                      </EmptyDescription>
                    </EmptyHeader>
                  </Empty>
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      )}

      {data && data.totalPages > 1 ? (
        <div className="flex justify-center py-2">
          <Pagination
            count={data.totalPages}
            page={page}
            onPageChange={(next) =>
              void navigate({ search: (prev) => ({ ...prev, page: next > 1 ? next : undefined }), replace: true })
            }
            label="Audit log pages"
          />
        </div>
      ) : null}
    </SettingsPage>
  );
}

type AuditLogSearchPatch = {
  action?: string;
  from?: string;
  to?: string;
};
