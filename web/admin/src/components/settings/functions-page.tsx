import { useQuery } from "@tanstack/react-query";
import { Code2, FileCode2, Route as RouteIcon, RotateCw } from "lucide-react";
import { cb } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { StatCard } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsPage } from "@/components/settings/settings-form";
import { TimeAgo } from "@/components/settings/time-ago";

/** As `GET /api/functions` returns them. */
type HookFile = { name: string; sizeBytes: number; modifiedAt: string | null };
type HookRoute = { method: string; pattern: string };
type FunctionsResponse = { hooksDir: string; files: HookFile[]; routes: HookRoute[] };

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Read-only view over `cb_hooks/`: which `*.cb.js` files the server found
 * at startup and which HTTP routes they registered via `routerAdd`. There
 * is no editor here on purpose — hooks are edited as files on disk (see
 * `ARCHITECTURE.md`'s "Extending with JavaScript" section), so this page
 * only ever reflects what is already on the server, not something the
 * dashboard can change.
 */
export function FunctionsPage() {
  const { data, isPending, isFetching, error, refetch } = useQuery({
    queryKey: ["functions"],
    queryFn: () => cb.send<FunctionsResponse>("/api/functions", { method: "GET" }),
  });

  const hooksDir = data?.hooksDir ?? "cb_hooks";
  const files = data?.files ?? [];
  const routes = data?.routes ?? [];

  const item = settingsItemFor("/settings/functions")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Button variant="outline" size="sm" disabled={isFetching} onClick={() => void refetch()}>
          {isFetching ? <Spinner /> : <RotateCw />}
          Refresh
        </Button>
      }
    >
      {error ? (
        <ErrorState title="Couldn't load functions" error={error} onRetry={() => void refetch()} compact />
      ) : isPending ? (
        <>
          <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
            <StatCard label="Hook files" value={0} loading />
            <StatCard label="HTTP routes" value={0} loading />
          </div>
          <ListSkeleton rows={4} />
        </>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
            <StatCard
              index={0}
              label="Hook files"
              icon={<FileCode2 />}
              value={files.length}
              hint={<span className="font-mono">{hooksDir}/</span>}
            />
            <StatCard
              index={1}
              label="HTTP routes"
              icon={<RouteIcon />}
              value={routes.length}
              hint="Registered with routerAdd"
            />
          </div>

          <section className="flex flex-col gap-2" aria-labelledby="hook-files-heading">
            <div>
              <h3 id="hook-files-heading" className="text-sm font-medium">
                Hook files
              </h3>
              <p className="max-w-measure text-xs text-muted-foreground">
                Loaded from <code className="font-mono">{hooksDir}/</code> when the server starts. Edit them on disk and
                restart to apply changes.
              </p>
            </div>
            {files.length === 0 ? (
              <EmptyState
                compact
                icon={<Code2 />}
                title="No hook files yet"
                description={
                  <>
                    Create a <code className="font-mono">*.cb.js</code> file in{" "}
                    <code className="font-mono">{hooksDir}/</code> next to the server&apos;s data directory, then
                    restart. A hook file can register record hooks, cron jobs (<code className="font-mono">cronAdd</code>),
                    queue handlers (<code className="font-mono">onQueueJob</code>) and HTTP routes (
                    <code className="font-mono">routerAdd</code>).
                  </>
                }
              />
            ) : (
              <div className="rounded-lg border border-border bg-card overflow-hidden">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>File</TableHead>
                      <TableHead className="w-28">Size</TableHead>
                      <TableHead className="w-40">Last modified</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {files.map((file) => (
                      <TableRow key={file.name}>
                        <TableCell className="py-2 font-mono text-sm">{file.name}</TableCell>
                        <TableCell className="font-tabular py-2 font-mono text-xs text-muted-foreground">
                          {formatSize(file.sizeBytes)}
                        </TableCell>
                        <TableCell className="py-2 text-xs text-muted-foreground">
                          <TimeAgo value={file.modifiedAt} />
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </section>

          <section className="flex flex-col gap-2" aria-labelledby="hook-routes-heading">
            <div>
              <h3 id="hook-routes-heading" className="text-sm font-medium">
                Custom routes
              </h3>
              <p className="max-w-measure text-xs text-muted-foreground">
                Root-level HTTP routes registered by a hook file&apos;s own{" "}
                <code className="font-mono">routerAdd(method, path, handler)</code> calls — not nested under{" "}
                <code className="font-mono">/api</code>.
              </p>
            </div>
            {routes.length === 0 ? (
              <EmptyState
                compact
                icon={<RouteIcon />}
                title="No custom routes"
                description={
                  <>
                    Call <code className="font-mono">routerAdd(&quot;GET&quot;, &quot;/hello&quot;, handler)</code> in
                    a <code className="font-mono">{hooksDir}/*.cb.js</code> file to expose an endpoint.
                  </>
                }
              />
            ) : (
              <div className="rounded-lg border border-border bg-card overflow-hidden">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead className="w-24">Method</TableHead>
                      <TableHead>Path</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {routes.map((route, i) => (
                      <TableRow key={`${route.method}-${route.pattern}-${i}`}>
                        <TableCell className="py-2">
                          <Badge variant="secondary" className="font-mono text-2xs">
                            {route.method}
                          </Badge>
                        </TableCell>
                        <TableCell className="py-2 font-mono text-sm">{route.pattern}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </section>
        </>
      )}
    </SettingsPage>
  );
}
