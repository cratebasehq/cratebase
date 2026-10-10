import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, FunctionSquare, Pencil, Play, Plus, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import CodeMirror from "@uiw/react-codemirror";
import { sql, PostgreSQL, type SQLNamespace } from "@codemirror/lang-sql";
import { useTheme } from "next-themes";
import { cb, describeFailure } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { settingsItemFor } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { RuleField } from "@/components/collections/rule-field";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { StatusBadge } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { SettingsPage } from "@/components/settings/settings-form";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
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

/** Same rule the server enforces on `name` and each parameter name. */
const IDENTIFIER = /^[A-Za-z_][A-Za-z0-9_]*$/;

/**
 * CRUD for `_rpc` (custom SQL RPC definitions) plus a "Run" panel that
 * calls `POST /api/rpc/{name}` the same way an application would. See
 * `crates/server/src/rpc.rs`'s module doc for the full security model —
 * this page edits the exact same trust-tier object the SQL console and
 * cron jobs pages do.
 */
export function RpcFunctionsPage() {
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [dialogDef, setDialogDef] = useState<RpcDefinition | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RpcDefinition | null>(null);
  const [runTarget, setRunTarget] = useState<RpcDefinition | null>(null);

  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["rpc-definitions"],
    queryFn: () => cb.collection("_rpc").fullList({ sort: "-created" }) as unknown as Promise<RpcDefinition[]>,
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_rpc").delete(id),
    onSuccess: () => {
      toast.success(`Deleted ${deleteTarget?.name ?? "RPC function"}`);
      void queryClient.invalidateQueries({ queryKey: ["rpc-definitions"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't delete the RPC function", { description: described.detail });
    },
  });

  const defs = data ?? [];
  const needle = search.trim().toLowerCase();
  const visible = defs.filter(
    (def) => !needle || def.name.toLowerCase().includes(needle) || def.sql.toLowerCase().includes(needle),
  );
  const item = settingsItemFor("/settings/rpc")!;

  const newButton = (
    <Button size="sm" onClick={() => setDialogDef("new")}>
      <Plus />
      New RPC function
    </Button>
  );

  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      {defs.length > 0 ? (
        <DataTableToolbar
          search={{ value: search, onChange: setSearch, placeholder: "Search functions…", label: "Search RPC functions" }}
          actions={newButton}
        />
      ) : null}

      {error ? (
        <ErrorState title="Couldn't load RPC functions" error={error} onRetry={() => void refetch()} compact />
      ) : isPending ? (
        <ListSkeleton rows={4} />
      ) : defs.length === 0 ? (
        <EmptyState
          compact
          icon={<FunctionSquare />}
          title="No RPC functions yet"
          description={
            <>
              Save a named SQL statement and call it from your app as{" "}
              <code className="font-mono">POST /api/rpc/&#123;name&#125;</code> — with typed parameters, its own access
              rule, a timeout and a row cap.
            </>
          }
          action={newButton}
        />
      ) : visible.length === 0 ? (
        <EmptyState
          compact
          icon={<FunctionSquare />}
          title="No functions match"
          description={`Nothing matches "${search.trim()}".`}
          action={
            <Button variant="outline" size="sm" onClick={() => setSearch("")}>
              Clear search
            </Button>
          }
        />
      ) : (
        <div className="rounded-lg border border-border bg-card overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Function</TableHead>
                <TableHead>Who may call it</TableHead>
                <TableHead>Mode</TableHead>
                <TableHead>Limits</TableHead>
                <TableHead className="w-32 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.map((def) => {
                const paramNames = (def.params ?? []).map((p) => p.name).join(", ");
                return (
                  <TableRow key={def.id} className="align-top">
                    <TableCell className="py-2.5">
                      <div className="font-mono text-sm font-medium">
                        {def.name}
                        <span className="font-normal text-muted-foreground">({paramNames})</span>
                      </div>
                      <div className="mt-0.5 max-w-sm truncate font-mono text-xs text-muted-foreground" title={def.sql}>
                        {def.sql}
                      </div>
                    </TableCell>
                    <TableCell className="py-2.5 text-sm">
                      {def.rule === null ? (
                        <StatusBadge tone="neutral">Superusers only</StatusBadge>
                      ) : def.rule === "" ? (
                        <StatusBadge tone="warning">Anyone</StatusBadge>
                      ) : (
                        <div className="flex flex-col gap-0.5">
                          <StatusBadge tone="info" className="w-fit">
                            Rule
                          </StatusBadge>
                          <code className="max-w-48 truncate font-mono text-xs" title={def.rule}>
                            {def.rule}
                          </code>
                        </div>
                      )}
                    </TableCell>
                    <TableCell className="py-2.5">
                      {def.readOnly ? (
                        <StatusBadge tone="success">Read-only</StatusBadge>
                      ) : (
                        <StatusBadge tone="warning">Can write</StatusBadge>
                      )}
                    </TableCell>
                    <TableCell className="font-tabular py-2.5 font-mono text-xs text-muted-foreground">
                      {def.timeoutMs} ms · {def.maxRows} rows
                    </TableCell>
                    <TableCell className="py-2.5 text-right">
                      <div className="flex justify-end gap-0.5">
                        <Button variant="ghost" size="xs" aria-label={`Test ${def.name}`} onClick={() => setRunTarget(def)}>
                          <Play />
                          Test
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          aria-label={`Edit ${def.name}`}
                          onClick={() => setDialogDef(def)}
                        >
                          <Pencil />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          aria-label={`Delete ${def.name}`}
                          onClick={() => setDeleteTarget(def)}
                        >
                          <Trash2 />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
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

      <TypedConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title={`Delete ${deleteTarget?.name ?? "function"}?`}
        description={
          <>
            <code className="font-mono">POST /api/rpc/{deleteTarget?.name}</code> starts returning 404 immediately, so
            any app still calling it will break. There is no undo.
          </>
        }
        confirmValue={deleteTarget?.name ?? ""}
        confirmLabel="Delete function"
        pendingLabel="Deleting…"
        pending={remove.isPending}
        onConfirm={() => deleteTarget && remove.mutate(deleteTarget.id)}
      />
    </SettingsPage>
  );
}

/** `:name` placeholders in a SQL statement, ignoring `::casts`. A hint for the
 * editor only — the server's own parser is the authority at save time. */
function referencedPlaceholders(statement: string): string[] {
  const found = new Set<string>();
  for (const match of statement.matchAll(/(?<![:\w]):([A-Za-z_][A-Za-z0-9_]*)/g)) found.add(match[1]);
  return [...found];
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
  const [serverErrors, setServerErrors] = useState<Record<string, string>>({});
  const [submitted, setSubmitted] = useState(false);

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
      toast.success(def ? `Updated ${name}` : `Created ${name}`);
      void queryClient.invalidateQueries({ queryKey: ["rpc-definitions"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setServerErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(def ? "Couldn't update the RPC function" : "Couldn't create the RPC function", {
          description: described.detail,
        });
      }
    },
  });

  function updateParam(index: number, patch: Partial<RpcParamSpec>) {
    setServerErrors((prev) => ({ ...prev, params: "" }));
    setParams((prev) => prev.map((p, i) => (i === index ? { ...p, ...patch } : p)));
  }

  // Client-side checks mirror the server's, so most mistakes show up inline
  // before a round trip; anything the server still rejects lands in the same slots.
  const errors: Record<string, string> = {};
  if (name.trim() === "") errors["name"] = "Give the function a name.";
  else if (!IDENTIFIER.test(name)) {
    errors["name"] = "Use letters, digits and underscores only, and don't start with a digit.";
  }
  if (sqlText.trim() === "") errors["sql"] = "Enter the SQL statement to run.";
  const seen = new Set<string>();
  for (const p of params) {
    if (!IDENTIFIER.test(p.name)) {
      errors["params"] = "Every parameter needs a name: letters, digits and underscores, not starting with a digit.";
      break;
    }
    if (seen.has(p.name)) {
      errors["params"] = `Parameter "${p.name}" is declared twice.`;
      break;
    }
    seen.add(p.name);
  }
  if (!(Number.isInteger(Number(timeoutMs)) && Number(timeoutMs) > 0)) errors["timeoutMs"] = "Whole number of milliseconds, above 0.";
  if (!(Number.isInteger(Number(maxRows)) && Number(maxRows) > 0)) errors["maxRows"] = "Whole number of rows, above 0.";

  const errorFor = (field: string) => serverErrors[field] || (submitted ? errors[field] : undefined);

  const referenced = referencedPlaceholders(sqlText);
  const declared = params.map((p) => p.name).filter(Boolean);
  const undeclared = referenced.filter((r) => !declared.includes(r));
  const unused = declared.filter((d) => !referenced.includes(d));

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] flex-col sm:max-w-2xl">
        <form
          noValidate
          className="flex min-h-0 flex-1 flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            setSubmitted(true);
            if (Object.keys(errors).length > 0) return;
            setServerErrors({});
            save.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>{def ? `Edit ${def.name}` : "New RPC function"}</DialogTitle>
            <DialogDescription>
              Runs as a superuser-authored statement, gated per call by its own rule below — not by any collection's
              rules.
            </DialogDescription>
          </DialogHeader>

          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pr-1">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="rpc-name">Name</Label>
              <Input
                id="rpc-name"
                autoFocus
                className="font-mono"
                value={name}
                aria-invalid={errorFor("name") ? true : undefined}
                aria-describedby="rpc-name-help"
                onChange={(e) => {
                  setName(e.target.value);
                  setServerErrors((prev) => ({ ...prev, name: "" }));
                }}
                placeholder="nearest_stores"
                autoComplete="off"
                spellCheck={false}
              />
              {errorFor("name") ? (
                <p id="rpc-name-help" role="alert" className="text-xs text-destructive">
                  {errorFor("name")}
                </p>
              ) : (
                <p id="rpc-name-help" className="text-xs text-muted-foreground">
                  Becomes <code className="font-mono">POST /api/rpc/{name || "{name}"}</code>.
                </p>
              )}
            </div>

            <div className="flex flex-col gap-1.5">
              <Label id="rpc-sql-label">SQL</Label>
              <div role="group" aria-labelledby="rpc-sql-label" aria-describedby="rpc-sql-help">
                <CodeMirror
                  value={sqlText}
                  onChange={(value) => {
                    setSqlText(value);
                    setServerErrors((prev) => ({ ...prev, sql: "" }));
                  }}
                  theme={resolvedTheme === "light" ? "light" : "dark"}
                  extensions={extensions}
                  placeholder="SELECT * FROM stores WHERE geoDistance(loc.lon, loc.lat, :lon, :lat) < :radiusKm"
                  minHeight="6rem"
                  basicSetup={{ foldGutter: false, highlightActiveLine: false }}
                  className={cn(
                    "overflow-hidden rounded-md border text-sm [&_.cm-placeholder]:italic",
                    errorFor("sql") ? "border-destructive" : "border-border",
                  )}
                />
              </div>
              {errorFor("sql") ? (
                <p role="alert" className="text-xs text-destructive">
                  {errorFor("sql")}
                </p>
              ) : null}
              <p id="rpc-sql-help" className="text-xs text-muted-foreground">
                A single statement. Use <code className="font-mono">:name</code> placeholders for each parameter
                declared below.
              </p>
              {undeclared.length > 0 ? (
                <p className="text-xs text-warning">
                  Uses <span className="font-mono">{undeclared.map((p) => `:${p}`).join(", ")}</span> without declaring{" "}
                  {undeclared.length === 1 ? "it" : "them"} below.
                </p>
              ) : null}
              {unused.length > 0 ? (
                <p className="text-xs text-warning">
                  Declared but never used in the SQL: <span className="font-mono">{unused.join(", ")}</span>.
                </p>
              ) : null}
            </div>

            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <Label>Parameters</Label>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => setParams((prev) => [...prev, { name: "", type: "text", required: false }])}
                >
                  <Plus />
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
                        aria-label={`Parameter ${i + 1} name`}
                        className="h-control-sm min-w-0 flex-1 font-mono text-xs"
                        value={p.name}
                        aria-invalid={submitted && !IDENTIFIER.test(p.name) ? true : undefined}
                        onChange={(e) => updateParam(i, { name: e.target.value })}
                        placeholder="paramName"
                        autoComplete="off"
                        spellCheck={false}
                      />
                      <Select value={p.type} onValueChange={(v) => updateParam(i, { type: v as ParamType })}>
                        <SelectTrigger size="sm" aria-label={`Parameter ${i + 1} type`} className="w-28 text-xs">
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
                        aria-label={`Remove parameter ${p.name || i + 1}`}
                        onClick={() => setParams((prev) => prev.filter((_, j) => j !== i))}
                      >
                        <X />
                      </Button>
                    </div>
                  ))}
                </div>
              )}
              {errorFor("params") ? (
                <p role="alert" className="text-xs text-destructive">
                  {errorFor("params")}
                </p>
              ) : null}
            </div>

            <RuleField label="Who may call it" value={rule} onChange={setRule} />

            <div className="flex flex-col gap-2 rounded-lg border border-border p-3">
              <div className="flex items-start gap-2">
                <Switch id="rpc-read-only" className="mt-0.5" checked={readOnly} onCheckedChange={setReadOnly} />
                <div className="flex flex-col">
                  <Label htmlFor="rpc-read-only">Read-only</Label>
                  <p className="text-xs text-muted-foreground">
                    Runs inside a read-only transaction the database itself enforces (a disguised write is rejected,
                    not just discouraged). Turn it off only for a statement that needs to write.
                  </p>
                </div>
              </div>
              <div className="flex flex-wrap gap-4">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="rpc-timeout">Timeout (ms)</Label>
                  <Input
                    id="rpc-timeout"
                    type="number"
                    min={1}
                    inputMode="numeric"
                    className="font-tabular w-32 font-mono"
                    value={timeoutMs}
                    aria-invalid={errorFor("timeoutMs") ? true : undefined}
                    aria-describedby={errorFor("timeoutMs") ? "rpc-timeout-error" : undefined}
                    onChange={(e) => setTimeoutMs(e.target.value)}
                  />
                  {errorFor("timeoutMs") ? (
                    <p id="rpc-timeout-error" role="alert" className="max-w-40 text-xs text-destructive">
                      {errorFor("timeoutMs")}
                    </p>
                  ) : null}
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="rpc-max-rows">Max rows</Label>
                  <Input
                    id="rpc-max-rows"
                    type="number"
                    min={1}
                    inputMode="numeric"
                    className="font-tabular w-32 font-mono"
                    value={maxRows}
                    aria-invalid={errorFor("maxRows") ? true : undefined}
                    aria-describedby={errorFor("maxRows") ? "rpc-max-rows-error" : undefined}
                    onChange={(e) => setMaxRows(e.target.value)}
                  />
                  {errorFor("maxRows") ? (
                    <p id="rpc-max-rows-error" role="alert" className="max-w-40 text-xs text-destructive">
                      {errorFor("maxRows")}
                    </p>
                  ) : null}
                </div>
              </div>
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {save.isPending ? <Spinner className="size-3.5" /> : null}
              {def ? "Save" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function defaultParamValue(p: RpcParamSpec): string {
  if (p.default !== undefined && p.default !== null) {
    return typeof p.default === "object" ? JSON.stringify(p.default) : String(p.default);
  }
  if (p.type === "bool") return "false";
  return "";
}

/** The inline error for one parameter's raw text, or `null` when it is fine
 * to send (an empty optional parameter is simply omitted). */
function paramError(p: RpcParamSpec, raw: string): string | null {
  if (raw.trim() === "") return p.required ? "Required." : null;
  if (p.type === "number" && Number.isNaN(Number(raw))) return "Not a number.";
  if (p.type === "json") {
    try {
      JSON.parse(raw);
    } catch (failure) {
      return `Invalid JSON: ${failure instanceof Error ? failure.message : "could not parse"}`;
    }
  }
  return null;
}

function coerceParamValue(p: RpcParamSpec, raw: string): unknown {
  if (raw === "" && !p.required) return undefined;
  switch (p.type) {
    case "number":
      return Number(raw);
    case "bool":
      return raw === "true";
    case "json":
      return JSON.parse(raw);
    default:
      return raw;
  }
}

type ResultView = "table" | "json";

/** Test panel: one input per declared parameter, a Run button that calls
 * `cb.rpc` (the same SDK method any application uses), and the raw
 * `items` back. */
function RunRpcDialog({ def, onOpenChange }: { def: RpcDefinition; onOpenChange: (open: boolean) => void }) {
  const params = def.params ?? [];
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(params.map((p) => [p.name, defaultParamValue(p)])),
  );
  const [touched, setTouched] = useState<Record<string, boolean>>({});
  const [submitted, setSubmitted] = useState(false);
  const [view, setView] = useState<ResultView>("table");
  const { copy, copied } = useCopyToClipboard();

  const run = useMutation({
    mutationFn: async () => {
      const body: Record<string, unknown> = {};
      for (const p of params) {
        const coerced = coerceParamValue(p, values[p.name] ?? "");
        if (coerced !== undefined) body[p.name] = coerced;
      }
      const started = performance.now();
      const result = await cb.rpc(def.name, body);
      return { result, ms: Math.round(performance.now() - started) };
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't run ${def.name}`, { description: described.detail });
    },
  });

  const errors: Record<string, string> = {};
  for (const p of params) {
    const message = paramError(p, values[p.name] ?? "");
    if (message) errors[p.name] = message;
  }
  const showError = (name: string) => (submitted || touched[name] ? errors[name] : undefined);

  const items = (run.data?.result.items ?? []) as Record<string, unknown>[];
  const columns = items.length > 0 ? Object.keys(items[0]) : [];
  const resultJson = run.data ? JSON.stringify(run.data.result, null, 2) : "";
  const failure = run.isError ? describeFailure(run.error) : null;

  function setValue(name: string, value: string) {
    setValues((prev) => ({ ...prev, [name]: value }));
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] flex-col sm:max-w-2xl">
        <form
          noValidate
          className="flex min-h-0 flex-1 flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            setSubmitted(true);
            if (Object.keys(errors).length > 0 || run.isPending) return;
            run.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>
              Test <code className="font-mono">{def.name}</code>
            </DialogTitle>
            <DialogDescription>
              Calls <code className="font-mono">POST /api/rpc/{def.name}</code> as your current superuser session —
              same as any other caller would, subject to the same rule.
              {def.readOnly ? null : " This function can write: a successful run changes data."}
            </DialogDescription>
          </DialogHeader>

          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-1">
            {params.length === 0 ? (
              <p className="text-sm text-muted-foreground">This function takes no parameters.</p>
            ) : (
              params.map((p, i) => {
                const error = showError(p.name);
                const id = `rpc-run-${i}`;
                return (
                  <div key={p.name} className="flex flex-col gap-1.5">
                    <Label htmlFor={id} className="font-mono text-xs">
                      {p.name}
                      {p.required ? <span className="text-destructive"> *</span> : null}
                      <span className="ml-1.5 font-sans font-normal text-muted-foreground">({p.type})</span>
                    </Label>
                    {p.type === "bool" ? (
                      <Select value={values[p.name] ?? "false"} onValueChange={(v) => setValue(p.name, v)}>
                        <SelectTrigger id={id} size="sm" className="w-32">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="true">true</SelectItem>
                          <SelectItem value="false">false</SelectItem>
                        </SelectContent>
                      </Select>
                    ) : (
                      <Input
                        id={id}
                        type={p.type === "number" ? "number" : "text"}
                        step={p.type === "number" ? "any" : undefined}
                        className="font-mono text-sm"
                        value={values[p.name] ?? ""}
                        aria-invalid={error ? true : undefined}
                        aria-describedby={error ? `${id}-error` : undefined}
                        onChange={(e) => setValue(p.name, e.target.value)}
                        onBlur={() => setTouched((prev) => ({ ...prev, [p.name]: true }))}
                        placeholder={p.type === "json" ? '{"key":"value"}' : p.type === "date" ? "2026-01-31" : undefined}
                        autoComplete="off"
                        spellCheck={false}
                      />
                    )}
                    {error ? (
                      <p id={`${id}-error`} role="alert" className="text-xs text-destructive">
                        {error}
                      </p>
                    ) : null}
                  </div>
                );
              })
            )}

            <section
              aria-label="Result"
              aria-live="polite"
              className="flex flex-col gap-2 rounded-lg border border-border bg-card p-3"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-medium">Result</h3>
                {run.isSuccess ? (
                  <div className="flex items-center gap-2">
                    <span className="font-tabular font-mono text-xs text-muted-foreground">
                      {items.length} {items.length === 1 ? "row" : "rows"} · {run.data.ms} ms
                    </span>
                    <div role="group" aria-label="Result format" className="flex rounded-md border border-border p-0.5">
                      {(["table", "json"] as const).map((v) => (
                        <button
                          key={v}
                          type="button"
                          aria-pressed={view === v}
                          onClick={() => setView(v)}
                          className={cn(
                            "h-control-xs rounded-sm px-2 text-xs transition-colors duration-fast ease-out-strong",
                            view === v ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
                          )}
                        >
                          {v === "table" ? "Table" : "JSON"}
                        </button>
                      ))}
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="xs"
                      onClick={() =>
                        void copy(resultJson).then((ok) =>
                          ok ? toast.success("Result copied") : toast.error("Couldn't copy the result"),
                        )
                      }
                    >
                      {copied ? <Check /> : <Copy />}
                      {copied ? "Copied" : "Copy"}
                    </Button>
                  </div>
                ) : null}
              </div>

              {run.isPending ? (
                <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
                  <Spinner className="size-4" />
                  Running…
                </div>
              ) : failure ? (
                <div role="alert" className="rounded-md bg-destructive/10 p-3 text-xs text-destructive">
                  <p className="font-medium">{failure.title}</p>
                  {failure.detail ? <p className="mt-0.5 font-mono break-words whitespace-pre-wrap">{failure.detail}</p> : null}
                </div>
              ) : !run.isSuccess ? (
                <p className="text-sm text-muted-foreground">Not run yet — fill in the parameters and press Run.</p>
              ) : view === "json" ? (
                <pre className="max-h-64 overflow-auto rounded-md border border-border bg-surface-sunken p-3 font-mono text-xs">
                  {resultJson}
                </pre>
              ) : items.length === 0 ? (
                <p className="text-sm text-muted-foreground">The statement ran and returned no rows.</p>
              ) : (
                <div className="max-h-64 overflow-auto rounded-md border border-border">
                  <Table>
                    <TableHeader className="sticky top-0 bg-surface-sunken/95 backdrop-blur-sm">
                      <TableRow className="hover:bg-transparent">
                        {columns.map((c) => (
                          <TableHead key={c} className="font-mono text-xs whitespace-nowrap">
                            {c}
                          </TableHead>
                        ))}
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {items.map((row, i) => (
                        // eslint-disable-next-line react/no-array-index-key -- ad-hoc result set, no stable id
                        <TableRow key={i}>
                          {columns.map((c) => {
                            const cell = row[c];
                            const text = cell !== null && typeof cell === "object" ? JSON.stringify(cell) : String(cell ?? "");
                            return (
                              <TableCell
                                key={c}
                                title={text}
                                className="max-w-xs truncate font-mono text-xs whitespace-nowrap"
                              >
                                {text}
                              </TableCell>
                            );
                          })}
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              )}
            </section>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Close
            </Button>
            <Button type="submit" disabled={run.isPending}>
              {run.isPending ? <Spinner className="size-3.5" /> : <Play />}
              Run
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
