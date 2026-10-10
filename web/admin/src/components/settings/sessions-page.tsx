import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { LogOut, Radio, X } from "lucide-react";
import { cb, describeFailure, superuserAuth } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatusBadge } from "@/components/layout/stat-card";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingsPage } from "@/components/settings/settings-form";
import { LIVE_INTERVAL_MS, LiveToggle } from "@/components/settings/live-toggle";
import { TimeAgo } from "@/components/settings/time-ago";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** As `_sessions` stores them (`crates/core/src/collection.rs`). Every
 * live token, across every auth collection, tracked as its own row —
 * that is what makes "revoke this one device" and "who is currently
 * signed in" possible at all. */
interface SessionRecord {
  id: string;
  collectionRef: string;
  recordRef: string;
  kind: string;
  fingerprint: string;
  ip: string;
  userAgent: string;
  expiresAt: string;
  lastSeenAt: string | null;
  revoked: boolean;
  created: string;
}

const HEAD = "h-control-md text-xs font-medium text-muted-foreground";
const CELL = "py-1.5";

/** `impersonation` rows are the impersonator's own device fingerprint
 * against the *target* record, not the target's usual device — labelled
 * distinctly so an operator doesn't mistake one for the target's real
 * session. */
function kindLabel(kind: string): string {
  if (kind === "impersonation") return "Impersonation";
  if (kind === "cookie") return "Cookie";
  return "Bearer";
}

/** Every live (unrevoked, unexpired) token across every auth collection.
 * Read-only aside from revoking — there is no rule-driven write path
 * for `_sessions`, only `crates/server/src/sessions.rs`'s own functions
 * called from the auth/session routes (see that collection's own doc
 * comment in `crates/core/src/collection.rs`). */
export function SessionsPage() {
  const queryClient = useQueryClient();
  const { data: collections } = useCollections();
  const [collectionFilter, setCollectionFilter] = useState<string>("all");
  const [search, setSearch] = useState("");
  const [live, setLive] = useState(true);
  const [revokeTarget, setRevokeTarget] = useState<SessionRecord | null>(null);
  const [revokingOthers, setRevokingOthers] = useState(false);

  const authCollections = (collections ?? []).filter((c) => c.type === "auth");

  const filterExpr = ["revoked = false", collectionFilter !== "all" ? `collectionRef = "${collectionFilter}"` : ""]
    .filter(Boolean)
    .join(" && ");

  // Poll while the tab is visible (react-query pauses in background tabs) and
  // no confirmation is open, so a row never vanishes under the cursor.
  const dialogOpen = revokeTarget !== null || revokingOthers;
  const refetchInterval = live && !dialogOpen ? LIVE_INTERVAL_MS : false;

  const { data, isPending, error, refetch, isFetching } = useQuery({
    queryKey: ["sessions", collectionFilter],
    queryFn: () =>
      cb.collection("_sessions").list({ filter: filterExpr, sort: "-created", perPage: 200 }) as unknown as Promise<{
        items: SessionRecord[];
        totalItems?: number;
      }>,
    placeholderData: (previous) => previous,
    refetchInterval,
  });

  // The server flags which of *your own* sessions made the request
  // (`SessionRow.current`) — the only reliable way to tell "this browser"
  // apart from your other devices.
  const own = useQuery({
    queryKey: ["sessions-own"],
    queryFn: () => superuserAuth.sessions.list(),
    refetchInterval,
  });
  const currentId = own.data?.find((s) => s.current)?.id;
  const myOtherCount = (own.data ?? []).filter((s) => !s.current).length;

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["sessions"] }).then(() =>
      queryClient.invalidateQueries({ queryKey: ["sessions-own"] }),
    );
  }

  const revoke = useMutation({
    mutationFn: (row: SessionRecord) => superuserAuth.sessions.revoke(row.id, { collection: row.collectionRef }),
    onSuccess: (_result, row) => {
      void invalidate();
      toast.success(row.id === currentId ? "Revoked this session — you'll be signed out" : "Session revoked");
    },
    onError: (err) => {
      const failure = describeFailure(err);
      toast.error("Couldn't revoke the session", { description: failure.serverMessage || failure.detail || undefined });
    },
  });

  const revokeOthers = useMutation({
    mutationFn: () => superuserAuth.sessions.revokeOthers(),
    onSuccess: (result) => {
      void invalidate();
      toast.success(
        result.revoked === 1 ? "Signed out 1 other session" : `Signed out ${result.revoked} other sessions`,
      );
    },
    onError: (err) => {
      const failure = describeFailure(err);
      toast.error("Couldn't revoke your other sessions", {
        description: failure.serverMessage || failure.detail || undefined,
      });
    },
  });

  const items = data?.items ?? [];
  const query = search.trim().toLowerCase();
  const rows = query
    ? items.filter((r) => [r.recordRef, r.ip, r.userAgent, r.kind].some((v) => v.toLowerCase().includes(query)))
    : items;
  const collectionName = (id: string) => authCollections.find((c) => c.id === id)?.name ?? id;
  const superuserCollection = authCollections.find((c) => c.name === "_superusers");
  // Ownership is only knowable for the superuser collection — it is the one
  // the signed-in account lives in.
  const isMine = (row: SessionRecord) =>
    row.recordRef === superuserAuth.record?.id && (!superuserCollection || row.collectionRef === superuserCollection.id);

  const item = settingsItemFor("/settings/sessions")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <DataTableToolbar
        search={{ value: search, onChange: setSearch, placeholder: "Search record, IP, user agent…" }}
        filters={
          <Select value={collectionFilter} onValueChange={setCollectionFilter}>
            <SelectTrigger id="sessions-collection-filter" aria-label="Collection" className="h-control-sm w-52">
              <SelectValue placeholder="Every auth collection" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">Every auth collection</SelectItem>
              {authCollections.map((c) => (
                <SelectItem key={c.id} value={c.id}>
                  {c.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        }
        actions={
          <>
            <LiveToggle
              live={live}
              onLiveChange={setLive}
              fetching={isFetching}
              onRefresh={() => {
                void refetch();
                void own.refetch();
              }}
            />
            <Button
              variant="outline"
              size="sm"
              disabled={myOtherCount === 0 || revokeOthers.isPending}
              title={myOtherCount === 0 ? "You have no other signed-in sessions" : undefined}
              onClick={() => setRevokingOthers(true)}
            >
              <LogOut />
              Sign out my other sessions
            </Button>
          </>
        }
      />

      {error ? (
        <ErrorState compact title="Couldn't load sessions" error={describeFailure(error).detail} onRetry={() => void refetch()} />
      ) : isPending ? (
        <ListSkeleton rows={5} />
      ) : rows.length === 0 ? (
        <EmptyState
          compact
          icon={<Radio />}
          title={query ? "No sessions match your search" : "No live sessions"}
          description={
            query
              ? "Try a different record id, IP address, or user agent."
              : "Every bearer or cookie session that has signed in and not yet expired or been revoked shows up here."
          }
          action={
            query ? (
              <Button variant="outline" size="sm" onClick={() => setSearch("")}>
                Clear search
              </Button>
            ) : undefined
          }
        />
      ) : (
        <div className="flex flex-col gap-2">
          <div className="overflow-hidden rounded-lg border border-border bg-card">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className={cn(HEAD, "pl-3")}>Record</TableHead>
                  <TableHead className={HEAD}>Kind</TableHead>
                  <TableHead className={HEAD}>IP</TableHead>
                  <TableHead className={HEAD}>User agent</TableHead>
                  <TableHead className={HEAD}>Last seen</TableHead>
                  <TableHead className={HEAD}>Expires</TableHead>
                  <TableHead className={cn(HEAD, "w-12 pr-3")}>
                    <span className="sr-only">Actions</span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((row) => {
                  const isCurrent = row.id === currentId;
                  return (
                    <TableRow key={row.id} data-current={isCurrent || undefined}>
                      <TableCell className={cn(CELL, "pl-3")}>
                        <div className="flex flex-col gap-0.5">
                          <span className="flex items-center gap-2">
                            <span className="font-mono text-xs">{row.recordRef}</span>
                            {isCurrent ? (
                              <StatusBadge tone="brand">This session</StatusBadge>
                            ) : isMine(row) ? (
                              <StatusBadge tone="neutral">Your account</StatusBadge>
                            ) : null}
                          </span>
                          <span className="font-mono text-2xs text-muted-foreground">
                            {collectionName(row.collectionRef)}
                          </span>
                        </div>
                      </TableCell>
                      <TableCell className={CELL}>
                        <StatusBadge tone={row.kind === "impersonation" ? "danger" : "neutral"}>
                          {kindLabel(row.kind)}
                        </StatusBadge>
                      </TableCell>
                      <TableCell className={cn(CELL, "font-mono text-xs text-muted-foreground")}>{row.ip || "—"}</TableCell>
                      <TableCell
                        className={cn(CELL, "max-w-xs truncate text-xs text-muted-foreground")}
                        title={row.userAgent}
                      >
                        {row.userAgent || "—"}
                      </TableCell>
                      <TableCell className={cn(CELL, "text-xs text-muted-foreground")}>
                        <TimeAgo value={row.lastSeenAt} />
                      </TableCell>
                      <TableCell className={cn(CELL, "text-xs text-muted-foreground")}>
                        <TimeAgo value={row.expiresAt} />
                      </TableCell>
                      <TableCell className={cn(CELL, "pr-3 text-right")}>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Revoke session for ${row.recordRef}${isCurrent ? " (this session)" : ""}`}
                          title="Revoke session"
                          className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                          onClick={() => setRevokeTarget(row)}
                        >
                          <X />
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </div>
          <p className="font-tabular text-xs text-muted-foreground">
            {rows.length} active {rows.length === 1 ? "session" : "sessions"}
            {data?.totalItems !== undefined && data.totalItems > items.length
              ? ` · showing the newest ${items.length} of ${data.totalItems}`
              : ""}
          </p>
        </div>
      )}

      <AlertDialog open={revokeTarget !== null} onOpenChange={(open) => !open && setRevokeTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Revoke this session?</AlertDialogTitle>
            <AlertDialogDescription>
              {revokeTarget?.id === currentId
                ? "This is the session you're using right now — you'll be signed out of the dashboard immediately."
                : "The device it belongs to is signed out immediately and must authenticate again to get a new one."}
              {revokeTarget ? (
                <>
                  {" "}
                  Record <span className="font-mono">{revokeTarget.recordRef}</span>
                  {revokeTarget.ip ? (
                    <>
                      {" "}
                      from <span className="font-mono">{revokeTarget.ip}</span>
                    </>
                  ) : null}
                  .
                </>
              ) : null}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = revokeTarget;
                setRevokeTarget(null);
                if (target) revoke.mutate(target);
              }}
            >
              Revoke session
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={revokingOthers} onOpenChange={setRevokingOthers}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Sign out your other sessions?</AlertDialogTitle>
            <AlertDialogDescription>
              {myOtherCount === 1 ? "1 other device" : `${myOtherCount} other devices`} signed in as you will be
              signed out immediately. This browser stays signed in.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setRevokingOthers(false);
                revokeOthers.mutate();
              }}
            >
              Sign out other sessions
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsPage>
  );
}
