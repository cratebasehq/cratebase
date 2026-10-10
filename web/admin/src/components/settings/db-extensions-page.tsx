import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CratebaseError } from "@cratebase/client";
import { ExternalLink, Plug, Puzzle } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { PageSection } from "@/components/layout/page";
import { StatusBadge } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
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
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { SettingsPage } from "@/components/settings/settings-form";

/** As `GET /api/db/extensions` returns them — see
 * `crates/server/src/routes/extensions.rs`. */
interface ExtensionInfo {
  name: string;
  defaultVersion: string | null;
  installedVersion: string | null;
  installed: boolean;
  schema: string | null;
  comment: string | null;
}

type StatusFilter = "all" | "installed" | "available";

/** A doc link for each extension this page calls out by name — every
 * other one `pg_available_extensions` reports still lists and installs
 * fine, just without a shortcut link. */
const DOCS_LINKS: Record<string, string> = {
  postgis: "https://postgis.net/documentation/",
  vector: "https://github.com/pgvector/pgvector#readme",
  pg_trgm: "https://www.postgresql.org/docs/current/pgtrgm.html",
  unaccent: "https://www.postgresql.org/docs/current/unaccent.html",
  citext: "https://www.postgresql.org/docs/current/citext.html",
  pg_stat_statements: "https://www.postgresql.org/docs/current/pgstatstatements.html",
  pgcrypto: "https://www.postgresql.org/docs/current/pgcrypto.html",
};

/** The popular extensions get their own section at the top, installed or
 * not, so they're never buried in the full `pg_available_extensions`
 * list (which is often 60+ entries most projects never touch). */
const POPULAR_ORDER = ["postgis", "vector", "pg_trgm", "unaccent", "citext", "pg_stat_statements", "pgcrypto"];

function isNotFound(error: unknown): boolean {
  return error instanceof CratebaseError && error.status === 404;
}

/**
 * Superuser-only Postgres extension management — CREATE/DROP EXTENSION
 * from the dashboard instead of a migration or the SQL console. Not
 * applicable at all on SQLite: `GET /api/db/extensions` 404s there, and
 * this page shows that plainly instead of an empty table (see
 * `crates/server/src/routes/extensions.rs`'s module doc for why there is
 * no SQLite equivalent to fall back to).
 */
export function DbExtensionsPage() {
  const queryClient = useQueryClient();
  const [installTarget, setInstallTarget] = useState<ExtensionInfo | null>(null);
  const [dropTarget, setDropTarget] = useState<ExtensionInfo | null>(null);
  const [cascade, setCascade] = useState(false);
  const [typedName, setTypedName] = useState("");
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<StatusFilter>("all");

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["db-extensions"],
    queryFn: () => cb.send<{ items: ExtensionInfo[] }>("/api/db/extensions", { method: "GET" }),
    retry: (failureCount, err) => !isNotFound(err) && failureCount < 2,
  });

  const install = useMutation({
    mutationFn: (name: string) => cb.send<void>(`/api/db/extensions/${encodeURIComponent(name)}`, { method: "POST" }),
    onSuccess: (_result, name) => {
      toast.success(`${name} installed`);
      void queryClient.invalidateQueries({ queryKey: ["db-extensions"] });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const drop = useMutation({
    mutationFn: ({ name, cascade }: { name: string; cascade: boolean }) =>
      cb.send<void>(`/api/db/extensions/${encodeURIComponent(name)}${cascade ? "?cascade=true" : ""}`, {
        method: "DELETE",
      }),
    onSuccess: (_result, { name }) => {
      toast.success(`${name} disabled`);
      void queryClient.invalidateQueries({ queryKey: ["db-extensions"] });
      closeDrop();
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  function closeDrop() {
    setDropTarget(null);
    setCascade(false);
    setTypedName("");
  }

  const items = useMemo(() => data?.items ?? [], [data]);
  const needle = search.trim().toLowerCase();
  const visible = useMemo(
    () =>
      items.filter((ext) => {
        if (status === "installed" && !ext.installed) return false;
        if (status === "available" && ext.installed) return false;
        return !needle || ext.name.toLowerCase().includes(needle) || (ext.comment ?? "").toLowerCase().includes(needle);
      }),
    [items, needle, status],
  );
  const installedCount = items.filter((ext) => ext.installed).length;

  const item = settingsItemFor("/settings/extensions")!;

  if (isNotFound(error)) {
    return (
      <SettingsPage title={item.label} description={item.description} width="wide">
        <EmptyState
          icon={<Plug />}
          title="Extensions need Postgres"
          description={
            <>
              This server is running on SQLite, which has no extension mechanism to manage. To use this page, point{" "}
              <code className="font-mono">DATABASE_URL</code> at a Postgres connection string such as{" "}
              <code className="font-mono">postgres://user:password@host:5432/dbname</code> and restart the server.
            </>
          }
        />
      </SettingsPage>
    );
  }

  const popular = POPULAR_ORDER.flatMap((name) => visible.filter((ext) => ext.name === name));
  const rest = visible.filter((ext) => !POPULAR_ORDER.includes(ext.name)).sort((a, b) => a.name.localeCompare(b.name));
  const filtering = needle !== "" || status !== "all";

  const table = (rows: ExtensionInfo[]) => (
    <ExtensionTable
      rows={rows}
      busyName={install.isPending ? install.variables : drop.isPending ? drop.variables?.name : undefined}
      onInstall={setInstallTarget}
      onDisable={setDropTarget}
    />
  );

  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      {error ? (
        <ErrorState title="Couldn't load extensions" error={error} onRetry={() => void refetch()} />
      ) : isLoading ? (
        <ListSkeleton rows={7} />
      ) : (
        <>
          <DataTableToolbar
            search={{ value: search, onChange: setSearch, placeholder: "Search extensions…" }}
            filters={
              <ToggleGroup
                type="single"
                variant="outline"
                size="sm"
                spacing={0}
                value={status}
                onValueChange={(value) => value && setStatus(value as StatusFilter)}
                aria-label="Filter by status"
              >
                <ToggleGroupItem value="all" className="px-2.5">
                  All <span className="font-tabular text-muted-foreground">{items.length}</span>
                </ToggleGroupItem>
                <ToggleGroupItem value="installed" className="px-2.5">
                  Installed <span className="font-tabular text-muted-foreground">{installedCount}</span>
                </ToggleGroupItem>
                <ToggleGroupItem value="available" className="px-2.5">
                  Available <span className="font-tabular text-muted-foreground">{items.length - installedCount}</span>
                </ToggleGroupItem>
              </ToggleGroup>
            }
          />

          {items.length === 0 ? (
            <EmptyState
              icon={<Puzzle />}
              title="No extensions available"
              description="This Postgres build reports no installable extensions. Install the contrib package (or the extension's own package) on the database host, then reload."
              action={
                <Button variant="outline" size="sm" onClick={() => void refetch()}>
                  Reload
                </Button>
              }
            />
          ) : visible.length === 0 ? (
            <EmptyState
              compact
              icon={<Puzzle />}
              title="No extensions match"
              description="Nothing fits the current search and filter."
              action={
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setSearch("");
                    setStatus("all");
                  }}
                >
                  Clear filters
                </Button>
              }
            />
          ) : (
            <div className="flex flex-col gap-section">
              {popular.length > 0 ? (
                <PageSection title="Popular" description="Commonly used extensions, pinned to the top.">
                  {table(popular)}
                </PageSection>
              ) : null}
              {rest.length > 0 ? (
                <PageSection
                  title={popular.length > 0 || !filtering ? "Everything else" : "Results"}
                  description="Every other extension this Postgres build has available."
                >
                  {table(rest)}
                </PageSection>
              ) : null}
            </div>
          )}
        </>
      )}

      <AlertDialog open={installTarget !== null} onOpenChange={(open) => !open && setInstallTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Enable “{installTarget?.name}”?</AlertDialogTitle>
            <AlertDialogDescription>
              Runs <code className="font-mono">CREATE EXTENSION</code>
              {installTarget?.defaultVersion ? ` (v${installTarget.defaultVersion})` : ""} in this database. The database
              role needs permission to create extensions; if it doesn't, Postgres will say so and nothing changes.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (installTarget) install.mutate(installTarget.name);
              }}
            >
              Enable
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <Dialog open={dropTarget !== null} onOpenChange={(open) => !open && !drop.isPending && closeDrop()}>
        <DialogContent className="sm:max-w-md">
          <form
            noValidate
            className="grid gap-4"
            onSubmit={(event) => {
              event.preventDefault();
              if (dropTarget && typedName === dropTarget.name && !drop.isPending) {
                drop.mutate({ name: dropTarget.name, cascade });
              }
            }}
          >
            <DialogHeader>
              <DialogTitle>Disable “{dropTarget?.name}”?</DialogTitle>
              <DialogDescription>
                Runs <code className="font-mono">DROP EXTENSION</code>. Postgres refuses this on its own if anything
                depends on it — tick “also drop dependent objects” to force it with{" "}
                <code className="font-mono">CASCADE</code> instead.
              </DialogDescription>
            </DialogHeader>
            <div className="flex items-start gap-2">
              <Checkbox
                id="extension-cascade"
                className="mt-0.5"
                checked={cascade}
                onCheckedChange={(checked) => setCascade(checked === true)}
              />
              <div className="flex flex-col">
                <Label htmlFor="extension-cascade" className="cursor-pointer text-sm font-normal">
                  Also drop dependent objects (CASCADE)
                </Label>
                <p className="text-xs text-muted-foreground">
                  Columns, indexes and functions that use this extension are dropped too. This cannot be undone.
                </p>
              </div>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="extension-confirm">
                Type <span className="font-mono font-semibold text-foreground">{dropTarget?.name}</span> to confirm
              </Label>
              <Input
                id="extension-confirm"
                autoFocus
                autoComplete="off"
                spellCheck={false}
                value={typedName}
                onChange={(event) => setTypedName(event.target.value)}
                className="font-mono"
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={closeDrop} disabled={drop.isPending}>
                Cancel
              </Button>
              <Button type="submit" variant="destructive" disabled={drop.isPending || typedName !== dropTarget?.name}>
                {drop.isPending ? <Spinner className="size-3.5" /> : null}
                Disable
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}

function ExtensionTable({
  rows,
  busyName,
  onInstall,
  onDisable,
}: {
  rows: ExtensionInfo[];
  busyName: string | undefined;
  onInstall: (ext: ExtensionInfo) => void;
  onDisable: (ext: ExtensionInfo) => void;
}) {
  return (
    <div className="overflow-hidden rounded-lg border border-border bg-card">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead>Extension</TableHead>
            <TableHead className="w-56">Status</TableHead>
            <TableHead className="w-28" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((ext) => {
            const pending = busyName === ext.name;
            const docs = DOCS_LINKS[ext.name];
            return (
              <TableRow key={ext.name} className="align-top">
                <TableCell className="py-2">
                  <div className="flex items-center gap-2 font-mono text-sm font-medium">
                    {ext.name}
                    {docs ? (
                      <a
                        href={docs}
                        target="_blank"
                        rel="noreferrer"
                        aria-label={`${ext.name} documentation`}
                        className="rounded-sm text-muted-foreground transition-colors duration-fast hover:text-foreground"
                      >
                        <ExternalLink className="size-3.5" />
                      </a>
                    ) : null}
                  </div>
                  {ext.comment ? <div className="text-xs text-pretty text-muted-foreground">{ext.comment}</div> : null}
                </TableCell>
                <TableCell className="py-2">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    {ext.installed ? (
                      <StatusBadge tone="success">Installed</StatusBadge>
                    ) : (
                      <StatusBadge tone="neutral">Available</StatusBadge>
                    )}
                    <span className="font-tabular font-mono text-xs text-muted-foreground">
                      {ext.installed
                        ? `v${ext.installedVersion}${ext.schema ? ` · ${ext.schema}` : ""}`
                        : ext.defaultVersion
                          ? `v${ext.defaultVersion}`
                          : ""}
                    </span>
                  </div>
                </TableCell>
                <TableCell className="py-1.5 text-right">
                  {ext.installed ? (
                    <Button variant="ghost" size="sm" disabled={pending} onClick={() => onDisable(ext)}>
                      {pending ? <Spinner /> : null}
                      Disable
                    </Button>
                  ) : (
                    <Button variant="outline" size="sm" disabled={pending} onClick={() => onInstall(ext)}>
                      {pending ? <Spinner /> : null}
                      Enable
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}
