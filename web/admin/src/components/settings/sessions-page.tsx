import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Radio, X } from "lucide-react";
import { cb, describeFailure, parseServerDate, superuserAuth } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { Badge } from "@/components/ui/badge";
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
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** As `_sessions` stores them (`crates/core/src/collection.rs`). Every
 * live token, across every auth collection, tracked as its own row —
 * that is what makes "revoke this one device" and "who is currently
 * signed in" possible at all; PocketBase has neither. */
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
 * Read-only aside from revoking one — there is no rule-driven write path
 * for `_sessions`, only `crates/server/src/sessions.rs`'s own functions
 * called from the auth/session routes (see that collection's own doc
 * comment in `crates/core/src/collection.rs`). */
export function SessionsPage() {
  const queryClient = useQueryClient();
  const { data: collections } = useCollections();
  const [collectionFilter, setCollectionFilter] = useState<string>("all");
  const [revokeTarget, setRevokeTarget] = useState<SessionRecord | null>(null);

  const authCollections = (collections ?? []).filter((c) => c.type === "auth");

  const filterExpr = ["revoked = false", collectionFilter !== "all" ? `collectionRef = "${collectionFilter}"` : ""]
    .filter(Boolean)
    .join(" && ");

  const { data, isLoading, error } = useQuery({
    queryKey: ["sessions", collectionFilter],
    queryFn: () =>
      cb.collection("_sessions").list({ filter: filterExpr, sort: "-created", perPage: 200 }) as unknown as Promise<{
        items: SessionRecord[];
      }>,
    placeholderData: (previous) => previous,
  });

  const revoke = useMutation({
    mutationFn: (row: SessionRecord) => superuserAuth.sessions.revoke(row.id, { collection: row.collectionRef }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
    onError: (err) => {
      const failure = describeFailure(err);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  const rows = data?.items ?? [];
  const collectionName = (id: string) => authCollections.find((c) => c.id === id)?.name ?? id;

  const item = settingsItemFor("/settings/sessions")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <div className="flex flex-wrap items-end gap-2">
        <div className="flex flex-col gap-1">
          <label htmlFor="sessions-collection-filter" className="text-xs text-muted-foreground">
            Collection
          </label>
          <Select value={collectionFilter} onValueChange={setCollectionFilter}>
            <SelectTrigger id="sessions-collection-filter" className="w-52">
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
        </div>
      </div>

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Radio />
            </EmptyMedia>
            <EmptyTitle>Couldn't load sessions</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[140px]">Collection</TableHead>
              <TableHead>Record</TableHead>
              <TableHead className="w-[110px]">Kind</TableHead>
              <TableHead className="w-[130px]">IP</TableHead>
              <TableHead>User agent</TableHead>
              <TableHead className="w-[160px]">Last seen</TableHead>
              <TableHead className="w-[160px]">Expires</TableHead>
              <TableHead className="w-[60px]" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.id}>
                <TableCell className="text-xs">{collectionName(row.collectionRef)}</TableCell>
                <TableCell className="font-mono text-xs">{row.recordRef}</TableCell>
                <TableCell>
                  <Badge variant={row.kind === "impersonation" ? "destructive" : "outline"} className="font-normal">
                    {kindLabel(row.kind)}
                  </Badge>
                </TableCell>
                <TableCell className="font-mono text-xs text-muted-foreground">{row.ip || "—"}</TableCell>
                <TableCell className="max-w-xs truncate text-xs text-muted-foreground" title={row.userAgent}>
                  {row.userAgent || "—"}
                </TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  {row.lastSeenAt ? parseServerDate(row.lastSeenAt).toLocaleString() : "—"}
                </TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  {row.expiresAt ? parseServerDate(row.expiresAt).toLocaleString() : "—"}
                </TableCell>
                <TableCell className="text-right">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Revoke session"
                    className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    onClick={() => setRevokeTarget(row)}
                  >
                    <X className="size-3.5" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
            {isLoading ? (
              Array.from({ length: 5 }).map((_, i) => (
                <TableRow key={i}>
                  <TableCell colSpan={8}>
                    <Skeleton className="h-row w-full" />
                  </TableCell>
                </TableRow>
              ))
            ) : null}
            {!isLoading && !error && rows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={8} className="py-8">
                  <Empty>
                    <EmptyHeader>
                      <EmptyMedia variant="icon">
                        <Radio />
                      </EmptyMedia>
                      <EmptyTitle>No live sessions</EmptyTitle>
                      <EmptyDescription>
                        Every bearer or cookie session that has signed in and not yet expired or been revoked will
                        show up here.
                      </EmptyDescription>
                    </EmptyHeader>
                  </Empty>
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      )}

      <AlertDialog open={revokeTarget !== null} onOpenChange={(open) => !open && setRevokeTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Revoke this session?</AlertDialogTitle>
            <AlertDialogDescription>
              The device it belongs to is signed out immediately and must authenticate again to get a new one.
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
              Revoke
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsPage>
  );
}
