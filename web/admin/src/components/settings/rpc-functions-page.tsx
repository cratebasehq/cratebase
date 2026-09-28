import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FunctionSquare, Pencil, Play, Plus, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import CodeMirror from "@uiw/react-codemirror";
import { sql, PostgreSQL, type SQLNamespace } from "@codemirror/lang-sql";
import { useTheme } from "next-themes";
import { cb, describeFailure } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { RuleField } from "@/components/collections/rule-field";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type ParamType = "text" | "number" | "bool" | "json" | "date";

interface RpcParamSpec {
  name: string;
  type: ParamType;
  required?: boolean;
  default?: unknown;
}

/** An `_rpc` record. `rule` displays as `null` even for a saved `""`
 * ("anyone") — see `crates/server/src/rpc.rs`'s module doc for why the
 * generic Records API can't distinguish that from `null` on the way
 * out; the field still round-trips correctly on *write*, so re-saving
 * an existing "anyone" rule as "anyone" here is harmless, it just can't
 * be *displayed* as already public without a fresh read of the raw row. */
interface RpcDefinition {
  id: string;
  name: string;
  sql: string;
  params: RpcParamSpec[] | null;
  rule: string | null;
  readOnly: boolean;
  timeoutMs: number;
  maxRows: number;
}

const PARAM_TYPES: ParamType[] = ["text", "number", "bool", "json", "date"];

/**
 * CRUD for `_rpc` (custom SQL RPC definitions) plus a "Run" panel that
 * calls `POST /api/rpc/{name}` the same way an application would. See
 * `crates/server/src/rpc.rs`'s module doc for the full security model —
 * this page edits the exact same trust-tier object the SQL console and
 * cron jobs pages do.
 */
export function RpcFunctionsPage() {
  const queryClient = useQueryClient();
  const [dialogDef, setDialogDef] = useState<RpcDefinition | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RpcDefinition | null>(null);
  const [runTarget, setRunTarget] = useState<RpcDefinition | null>(null);

  const { data, isLoading, error } = useQuery({
    queryKey: ["rpc-definitions"],
    queryFn: () => cb.collection("_rpc").fullList({ sort: "-created" }) as unknown as Promise<RpcDefinition[]>,
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_rpc").delete(id),
    onSuccess: () => {
      toast.success("RPC function deleted");
      void queryClient.invalidateQueries({ queryKey: ["rpc-definitions"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const defs = data ?? [];
  const item = settingsItemFor("/settings/rpc")!;

  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Button size="sm" className="gap-1.5" onClick={() => setDialogDef("new")}>
          <Plus className="size-3.5" />
          New RPC function
        </Button>
      }
    >
      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FunctionSquare />
            </EmptyMedia>
            <EmptyTitle>Couldn't load RPC functions</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : defs.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FunctionSquare />
            </EmptyMedia>
            <EmptyTitle>No RPC functions yet</EmptyTitle>
            <EmptyDescription>
              Save a named SQL statement to call as <code className="font-mono">POST /api/rpc/&#123;name&#125;</code>.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table className="max-w-5xl">
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Access</TableHead>
              <TableHead>Limits</TableHead>
              <TableHead className="w-36 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {defs.map((def) => (
              <TableRow key={def.id} className="align-top">
                <TableCell className="py-3">
                  <div className="flex items-center gap-2 font-mono text-sm font-medium">
                    {def.name}
                    {def.readOnly ? null : (
                      <Badge variant="outline" className="font-normal text-warning">
                        write
                      </Badge>
                    )}
                  </div>
                  <div className="mt-1 max-w-sm truncate font-mono text-[11px] text-muted-foreground/70">
                    {def.sql}
                  </div>
                </TableCell>
                <TableCell className="py-3 text-sm">
                  {def.rule === null ? "Superusers only" : def.rule === "" ? "Anyone" : <code className="font-mono text-xs">{def.rule}</code>}
                </TableCell>
                <TableCell className="py-3 text-xs text-muted-foreground">
                  {def.timeoutMs}ms · {def.maxRows} rows max
                </TableCell>
                <TableCell className="py-3 text-right">
                  <div className="flex justify-end gap-1">
                    <Button variant="ghost" size="sm" aria-label={`Run ${def.name}`} onClick={() => setRunTarget(def)}>
                      <Play className="size-3.5" />
                    </Button>
                    <Button variant="ghost" size="sm" aria-label={`Edit ${def.name}`} onClick={() => setDialogDef(def)}>
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Delete ${def.name}`}
                      onClick={() => setDeleteTarget(def)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {dialogDef ? (
        <RpcDefinitionDialog
          def={dialogDef === "new" ? null : dialogDef}
          onOpenChange={(open) => {
            if (!open) setDialogDef(null);
          }}
        />
      ) : null}

      {runTarget ? <RunRpcDialog def={runTarget} onOpenChange={(open) => !open && setRunTarget(null)} /> : null}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete "{deleteTarget?.name}"?</DialogTitle>
            <DialogDescription>
              <code className="font-mono">POST /api/rpc/{deleteTarget?.name}</code> starts 404ing immediately. There
              is no undo.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => deleteTarget && remove.mutate(deleteTarget.id)}
            >
              {remove.isPending ? <Spinner className="size-3.5" /> : null}
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}

/** Create or edit one `_rpc` record. `def === null` means create. */
function RpcDefinitionDialog({ def, onOpenChange }: { def: RpcDefinition | null; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient();
  const { resolvedTheme } = useTheme();
  const [name, setName] = useState(def?.name ?? "");
  const [sqlText, setSqlText] = useState(def?.sql ?? "");
  const [params, setParams] = useState<RpcParamSpec[]>(def?.params ?? []);
  const [rule, setRule] = useState<string | null>(def?.rule ?? null);
  const [readOnly, setReadOnly] = useState(def?.readOnly ?? true);
  const [timeoutMs, setTimeoutMs] = useState(String(def?.timeoutMs ?? 5000));
  const [maxRows, setMaxRows] = useState(String(def?.maxRows ?? 1000));
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const { data: collections } = useCollections();
  const schema: SQLNamespace = useMemo(() => {
    const namespace: Record<string, string[]> = {};
    for (const collection of collections ?? []) {
      namespace[collection.name] = (collection.fields ?? [])
        .map((field) => (typeof field.name === "string" ? field.name : null))
        .filter((n): n is string => n !== null);
    }
    return namespace;
  }, [collections]);
  const extensions = useMemo(() => [sql({ dialect: PostgreSQL, schema, upperCaseKeywords: true })], [schema]);

  const save = useMutation({
    mutationFn: () => {
      const body = {
        name,
        sql: sqlText,
        params,
        rule,
        readOnly,
        timeoutMs: Number(timeoutMs) || 5000,
        maxRows: Number(maxRows) || 1000,
      };
      return def ? cb.collection("_rpc").update(def.id, body) : cb.collection("_rpc").create(body);
    },
    onSuccess: () => {
      toast.success(def ? "RPC function updated" : "RPC function created");
      void queryClient.invalidateQueries({ queryKey: ["rpc-definitions"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(described.title, { description: described.detail });
      }
    },
  });

  function updateParam(index: number, patch: Partial<RpcParamSpec>) {
    setParams((prev) => prev.map((p, i) => (i === index ? { ...p, ...patch } : p)));
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] flex-col sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{def ? "Edit RPC function" : "New RPC function"}</DialogTitle>
          <DialogDescription>
            Runs as a superuser-authored statement, gated per call by its own rule below — not by any collection's
            rules.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4 overflow-y-auto pr-1">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="rpc-name">Name</Label>
            <Input
              id="rpc-name"
              className="font-mono"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="nearest_stores"
            />
            <p className="text-xs text-muted-foreground">
              Becomes <code className="font-mono">POST /api/rpc/{name || "{name}"}</code>.
            </p>
            {fieldErrors["name"] ? <p className="text-xs text-destructive">{fieldErrors["name"]}</p> : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label>SQL</Label>
            <CodeMirror
              value={sqlText}
              onChange={setSqlText}
              theme={resolvedTheme === "light" ? "light" : "dark"}
              extensions={extensions}
              placeholder="SELECT * FROM stores WHERE geoDistance(loc.lon, loc.lat, :lon, :lat) < :radiusKm"
              minHeight="6rem"
              basicSetup={{ foldGutter: false, highlightActiveLine: false }}
              className="overflow-hidden rounded-md border border-border/60 text-sm [&_.cm-placeholder]:italic"
            />
            <p className="text-xs text-muted-foreground">
              A single statement. Use <code className="font-mono">:name</code> placeholders for each parameter
              declared below.
            </p>
            {fieldErrors["sql"] ? <p className="text-xs text-destructive">{fieldErrors["sql"]}</p> : null}
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <Label>Parameters</Label>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="gap-1"
                onClick={() => setParams((prev) => [...prev, { name: "", type: "text", required: false }])}
              >
                <Plus className="size-3.5" />
                Add parameter
              </Button>
            </div>
            {params.length === 0 ? (
              <p className="text-xs text-muted-foreground">No declared parameters — this call takes none.</p>
            ) : (
              <div className="flex flex-col gap-2">
                {params.map((p, i) => (
                  // eslint-disable-next-line react/no-array-index-key -- reordered in place, no stable id yet
                  <div key={i} className="flex items-center gap-2">
                    <Input
                      className="h-control-sm min-w-0 flex-1 font-mono text-xs"
                      value={p.name}
                      onChange={(e) => updateParam(i, { name: e.target.value })}
                      placeholder="paramName"
                    />
                    <Select value={p.type} onValueChange={(v) => updateParam(i, { type: v as ParamType })}>
                      <SelectTrigger className="h-control-sm w-28 text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {PARAM_TYPES.map((t) => (
                          <SelectItem key={t} value={t}>
                            {t}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <div className="flex items-center gap-1.5 whitespace-nowrap text-xs text-muted-foreground">
                      <Checkbox
                        id={`rpc-param-required-${i}`}
                        checked={p.required ?? false}
                        onCheckedChange={(c) => updateParam(i, { required: c === true })}
                      />
                      <Label htmlFor={`rpc-param-required-${i}`} className="cursor-pointer font-normal">
                        required
                      </Label>
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Remove parameter ${i + 1}`}
                      onClick={() => setParams((prev) => prev.filter((_, j) => j !== i))}
                    >
                      <X className="size-3.5" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
            {fieldErrors["params"] ? <p className="text-xs text-destructive">{fieldErrors["params"]}</p> : null}
          </div>

          <RuleField label="Who may call it" value={rule} onChange={setRule} />

          <div className="flex flex-wrap items-end gap-4">
            <div className="flex items-center gap-2 pt-5">
              <Switch id="rpc-read-only" checked={readOnly} onCheckedChange={setReadOnly} />
              <Label htmlFor="rpc-read-only">Read-only</Label>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="rpc-timeout">Timeout (ms)</Label>
              <Input
                id="rpc-timeout"
                type="number"
                className="w-28"
                value={timeoutMs}
                onChange={(e) => setTimeoutMs(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="rpc-max-rows">Max rows</Label>
              <Input
                id="rpc-max-rows"
                type="number"
                className="w-28"
                value={maxRows}
                onChange={(e) => setMaxRows(e.target.value)}
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            Read-only runs inside a read-only transaction the database itself enforces (a disguised write is
            rejected, not just discouraged). Turn it off only for a statement that needs to write.
          </p>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={save.isPending || name.trim() === "" || sqlText.trim() === ""} onClick={() => save.mutate()}>
            {save.isPending ? <Spinner className="size-3.5" /> : null}
            {def ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function defaultParamValue(p: RpcParamSpec): string {
  if (p.default !== undefined && p.default !== null) return String(p.default);
  if (p.type === "bool") return "false";
  return "";
}

function coerceParamValue(p: RpcParamSpec, raw: string): unknown {
  if (raw === "" && !p.required) return undefined;
  switch (p.type) {
    case "number":
      return Number(raw);
    case "bool":
      return raw === "true";
    case "json":
      try {
        return JSON.parse(raw);
      } catch {
        return raw;
      }
    default:
      return raw;
  }
}

/** Test panel: one input per declared parameter, a Run button that calls
 * `cb.rpc` (the same SDK method any application uses), and the raw
 * `items` back. */
function RunRpcDialog({ def, onOpenChange }: { def: RpcDefinition; onOpenChange: (open: boolean) => void }) {
  const params = def.params ?? [];
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(params.map((p) => [p.name, defaultParamValue(p)])),
  );

  const run = useMutation({
    mutationFn: () => {
      const body: Record<string, unknown> = {};
      for (const p of params) {
        const coerced = coerceParamValue(p, values[p.name] ?? "");
        if (coerced !== undefined) body[p.name] = coerced;
      }
      return cb.rpc(def.name, body);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const items = run.data?.items ?? [];
  const columns = items.length > 0 ? Object.keys(items[0] as Record<string, unknown>) : [];

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] flex-col sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            Run <code className="font-mono">{def.name}</code>
          </DialogTitle>
          <DialogDescription>
            Calls <code className="font-mono">POST /api/rpc/{def.name}</code> as your current superuser session —
            same as any other caller would, subject to the same rule.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3 overflow-y-auto pr-1">
          {params.length === 0 ? (
            <p className="text-sm text-muted-foreground">This function takes no parameters.</p>
          ) : (
            params.map((p) => (
              <div key={p.name} className="flex flex-col gap-1.5">
                <Label htmlFor={`rpc-run-${p.name}`} className="font-mono text-xs">
                  {p.name}
                  {p.required ? <span className="text-destructive"> *</span> : null}
                  <span className="ml-1.5 font-sans font-normal text-muted-foreground">({p.type})</span>
                </Label>
                {p.type === "bool" ? (
                  <Select
                    value={values[p.name] ?? "false"}
                    onValueChange={(v) => setValues((prev) => ({ ...prev, [p.name]: v }))}
                  >
                    <SelectTrigger id={`rpc-run-${p.name}`} className="w-32">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="true">true</SelectItem>
                      <SelectItem value="false">false</SelectItem>
                    </SelectContent>
                  </Select>
                ) : (
                  <Input
                    id={`rpc-run-${p.name}`}
                    type={p.type === "number" ? "number" : "text"}
                    className="font-mono text-sm"
                    value={values[p.name] ?? ""}
                    onChange={(e) => setValues((prev) => ({ ...prev, [p.name]: e.target.value }))}
                    placeholder={p.type === "json" ? '{"key":"value"}' : undefined}
                  />
                )}
              </div>
            ))
          )}

          <div className="flex flex-col gap-2 rounded-lg border border-border bg-card p-3">
            <h3 className="text-sm font-medium">Result</h3>
            {run.isPending ? (
              <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
                <Spinner className="size-4" />
                Running…
              </div>
            ) : !run.isSuccess ? (
              <p className="text-sm text-muted-foreground">Not run yet.</p>
            ) : items.length === 0 ? (
              <p className="text-sm text-muted-foreground">No rows.</p>
            ) : (
              <div className="max-h-64 overflow-auto rounded-md border border-border">
                <Table>
                  <TableHeader className="sticky top-0 bg-surface-sunken/95 backdrop-blur-sm">
                    <TableRow className="hover:bg-transparent">
                      {columns.map((c) => (
                        <TableHead key={c} className="whitespace-nowrap font-mono text-xs">
                          {c}
                        </TableHead>
                      ))}
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {items.map((row, i) => (
                      // eslint-disable-next-line react/no-array-index-key -- ad-hoc result set, no stable id
                      <TableRow key={i}>
                        {columns.map((c) => (
                          <TableCell key={c} className="max-w-xs truncate whitespace-nowrap font-mono text-xs">
                            {String((row as Record<string, unknown>)[c] ?? "")}
                          </TableCell>
                        ))}
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Close
          </Button>
          <Button onClick={() => run.mutate()} disabled={run.isPending} className="gap-1.5">
            {run.isPending ? <Spinner className="size-3.5" /> : <Play className="size-3.5" />}
            Run
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
