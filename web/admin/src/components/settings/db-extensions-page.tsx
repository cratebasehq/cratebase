import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CratebaseError } from "@cratebase/client";
import { CheckCircle2, ExternalLink, Plug } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

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

/** The popular extensions get their own row at the top, installed or
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
  const [dropTarget, setDropTarget] = useState<ExtensionInfo | null>(null);
  const [cascade, setCascade] = useState(false);

  const { data, isLoading, error } = useQuery({
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
      setDropTarget(null);
      setCascade(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const item = settingsItemFor("/settings/extensions")!;

  if (isNotFound(error)) {
    return (
      <SettingsPage title={item.label} description={item.description} width="wide">
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Plug />
            </EmptyMedia>
            <EmptyTitle>Postgres only</EmptyTitle>
            <EmptyDescription>
              This server is running on SQLite, which has no extension mechanism. Switch{" "}
              <code className="font-mono">DATABASE_URL</code> to a Postgres connection string to use this page.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      </SettingsPage>
    );
  }

  const items = data?.items ?? [];
  const byName = new Map(items.map((e) => [e.name, e]));
  const popular = POPULAR_ORDER.map((name) => byName.get(name)).filter((e): e is ExtensionInfo => e !== undefined);
  const rest = items
    .filter((e) => !POPULAR_ORDER.includes(e.name))
    .sort((a, b) => a.name.localeCompare(b.name));

  function Row({ ext }: { ext: ExtensionInfo }) {
    const pending =
      (install.isPending && install.variables === ext.name) ||
      (drop.isPending && drop.variables?.name === ext.name);
    const docs = DOCS_LINKS[ext.name];
    return (
      <TableRow key={ext.name} className="align-top">
        <TableCell className="py-3">
          <div className="flex items-center gap-2 font-mono text-sm font-medium">
            {ext.name}
            {docs ? (
              <a
                href={docs}
                target="_blank"
                rel="noreferrer"
                aria-label={`${ext.name} documentation`}
                className="text-muted-foreground hover:text-foreground"
              >
                <ExternalLink className="size-3.5" />
              </a>
            ) : null}
          </div>
          {ext.comment ? <div className="text-xs text-muted-foreground">{ext.comment}</div> : null}
        </TableCell>
        <TableCell className="py-3 text-sm">
          {ext.installed ? (
            <div className="flex items-center gap-1.5">
              <Badge variant="outline" className="font-normal text-success">
                <CheckCircle2 className="size-3" />
                Installed
              </Badge>
              <span className="text-xs text-muted-foreground">
                v{ext.installedVersion}
                {ext.schema ? ` · ${ext.schema}` : ""}
              </span>
            </div>
          ) : (
            <span className="text-xs text-muted-foreground">
              Not installed{ext.defaultVersion ? ` (v${ext.defaultVersion} available)` : ""}
            </span>
          )}
        </TableCell>
        <TableCell className="py-3 text-right">
          {ext.installed ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={pending}
              onClick={() => setDropTarget(ext)}
            >
              {pending ? <Spinner className="size-3.5" /> : null}
              Disable
            </Button>
          ) : (
            <Button variant="outline" size="sm" disabled={pending} onClick={() => install.mutate(ext.name)}>
              {pending ? <Spinner className="size-3.5" /> : null}
              Enable
            </Button>
          )}
        </TableCell>
      </TableRow>
    );
  }

  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      {error && !isNotFound(error) ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Plug />
            </EmptyMedia>
            <EmptyTitle>Couldn't load extensions</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 5 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : (
        <div className="flex flex-col gap-6">
          <div className="flex flex-col gap-2">
            <h2 className="text-sm font-medium">Popular</h2>
            <Table className="max-w-3xl">
              <TableHeader>
                <TableRow>
                  <TableHead>Extension</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="w-24 text-right">&nbsp;</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {popular.map((ext) => (
                  <Row key={ext.name} ext={ext} />
                ))}
              </TableBody>
            </Table>
          </div>

          {rest.length > 0 ? (
            <div className="flex flex-col gap-2">
              <h2 className="text-sm font-medium">Everything else</h2>
              <p className="max-w-measure text-sm text-muted-foreground">
                Every other extension this Postgres build has available.
              </p>
              <Table className="max-w-3xl">
                <TableHeader>
                  <TableRow>
                    <TableHead>Extension</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead className="w-24 text-right">&nbsp;</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rest.map((ext) => (
                    <Row key={ext.name} ext={ext} />
                  ))}
                </TableBody>
              </Table>
            </div>
          ) : null}
        </div>
      )}

      <Dialog
        open={dropTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDropTarget(null);
            setCascade(false);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Disable "{dropTarget?.name}"?</DialogTitle>
            <DialogDescription>
              Runs <code className="font-mono">DROP EXTENSION</code>. Postgres refuses this on its own if
              anything depends on it — check "also drop dependent objects" to force it with{" "}
              <code className="font-mono">CASCADE</code> instead.
            </DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2">
            <Checkbox id="extension-cascade" checked={cascade} onCheckedChange={(c) => setCascade(c === true)} />
            <Label htmlFor="extension-cascade" className="cursor-pointer text-sm font-normal">
              Also drop dependent objects (CASCADE)
            </Label>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDropTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={drop.isPending}
              onClick={() => dropTarget && drop.mutate({ name: dropTarget.name, cascade })}
            >
              {drop.isPending ? <Spinner className="size-3.5" /> : null}
              Disable
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}
