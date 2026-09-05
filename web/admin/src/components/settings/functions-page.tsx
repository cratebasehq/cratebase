import { useQuery } from "@tanstack/react-query";
import { Code2, Route as RouteIcon } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { Badge } from "@/components/ui/badge";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** As `GET /api/functions` returns them. */
type HookFile = { name: string; sizeBytes: number; modifiedAt: string | null };
type HookRoute = { method: string; pattern: string };
type FunctionsResponse = { hooksDir: string; files: HookFile[]; routes: HookRoute[] };

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatModified(iso: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString();
}

/**
 * Read-only view over `pb_hooks/`: which `*.pb.js` files the server found
 * at startup and which HTTP routes they registered via `routerAdd`. There
 * is no editor here on purpose — hooks are edited as files on disk (see
 * `ARCHITECTURE.md`'s "Extending with JavaScript" section), so this page
 * only ever reflects what is already on the server, not something the
 * dashboard can change.
 */
export function FunctionsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["functions"],
    queryFn: () => cb.send<FunctionsResponse>("/api/functions", { method: "GET" }),
  });

  const hooksDir = data?.hooksDir ?? "pb_hooks";
  const files = data?.files ?? [];
  const routes = data?.routes ?? [];

  const item = settingsItemFor("/settings/functions")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="form">

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Code2 />
            </EmptyMedia>
            <EmptyTitle>Couldn't load functions</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : (
        <>
          <div className="flex flex-col gap-2">
            <h3 className="text-sm font-medium">Hook files</h3>
            {files.length === 0 ? (
              <Empty>
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <Code2 />
                  </EmptyMedia>
                  <EmptyTitle>No hook files</EmptyTitle>
                  <EmptyDescription>
                    Drop a <code className="font-mono text-xs">*.pb.js</code> file into{" "}
                    <code className="font-mono text-xs">{hooksDir}</code> to add one.
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : (
              <Table className="max-w-3xl">
                <TableHeader>
                  <TableRow>
                    <TableHead>File</TableHead>
                    <TableHead>Size</TableHead>
                    <TableHead>Last modified</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {files.map((file) => (
                    <TableRow key={file.name}>
                      <TableCell className="py-3 font-mono text-sm">{file.name}</TableCell>
                      <TableCell className="py-3 text-sm text-muted-foreground">{formatSize(file.sizeBytes)}</TableCell>
                      <TableCell className="py-3 text-sm text-muted-foreground">
                        {formatModified(file.modifiedAt)}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </div>

          <div className="flex flex-col gap-2">
            <h3 className="text-sm font-medium">Custom routes</h3>
            <p className="max-w-measure text-sm text-muted-foreground">
              Root-level HTTP routes registered by a hook file's own <code className="font-mono text-xs">routerAdd(method, path, handler)</code>{" "}
              calls — not nested under <code className="font-mono text-xs">/api</code>, same as PocketBase.
            </p>
            {routes.length === 0 ? (
              <Empty>
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <RouteIcon />
                  </EmptyMedia>
                  <EmptyTitle>No custom routes</EmptyTitle>
                  <EmptyDescription>No hook file has called `routerAdd` yet.</EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : (
              <Table className="max-w-3xl">
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-24">Method</TableHead>
                    <TableHead>Path</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {routes.map((route, i) => (
                    <TableRow key={`${route.method}-${route.pattern}-${i}`}>
                      <TableCell className="py-3">
                        <Badge variant="secondary" className="font-mono text-[11px]">
                          {route.method}
                        </Badge>
                      </TableCell>
                      <TableCell className="py-3 font-mono text-sm">{route.pattern}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </div>
        </>
      )}
    </SettingsPage>
  );
}
