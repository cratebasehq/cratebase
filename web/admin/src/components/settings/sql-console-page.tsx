import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTheme } from "next-themes";
import { AlertTriangle, BookmarkPlus, ChevronDown, Clock, Copy, Download, History, Play, Trash2, X } from "lucide-react";
import CodeMirror from "@uiw/react-codemirror";
import { sql, SQLite, type SQLNamespace } from "@codemirror/lang-sql";
import { keymap } from "@codemirror/view";
import { Prec } from "@codemirror/state";
import { cb, describeFailure } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { EmptyState } from "@/components/layout/states";
import { StatusBadge } from "@/components/layout/stat-card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { HoldToConfirm } from "@/components/ui/hold-to-confirm";
import { Input } from "@/components/ui/input";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsPage, ToggleSetting } from "@/components/settings/settings-form";

/** As `POST /api/sql` returns them — see `crate::routes::sql_console`. A
 * `SELECT`/`WITH` answers with `columns`/`rows`/`truncated`, anything else
 * with `rowsAffected`; the two shapes never share a field, so `"rowsAffected"
 * in result` tells them apart without a discriminant on the wire. */
type SqlReadResult = { columns: string[]; rows: Array<Record<string, unknown>>; truncated: boolean };
type SqlWriteResult = { rowsAffected: number };
type SqlResult = SqlReadResult | SqlWriteResult;
type RunOutcome = { result: SqlResult; ms: number };

function isWriteResult(result: SqlResult): result is SqlWriteResult {
  return "rowsAffected" in result;
}

const HISTORY_KEY = "cratebase.sql.history";
const SAVED_KEY = "cratebase.sql.saved";
const HISTORY_LIMIT = 30;
const EDITOR_MIN = 96;
const EDITOR_MAX = 560;

type HistoryEntry = { sql: string; at: number };
type SavedQuery = { name: string; sql: string };

function readList<T>(key: string): T[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(parsed) ? (parsed as T[]) : [];
  } catch {
    return [];
  }
}

function writeList<T>(key: string, value: T[]) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage unavailable or full: history just doesn't persist */
  }
}

/** Statements the server's read gate lets through without write mode. */
const READ_STATEMENT = /^(?:select|with|explain|pragma|values|show|table)$/;

function looksLikeWrite(text: string): boolean {
  const stripped = text.replace(/^(?:\s+|--[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\/)+/, "");
  const word = /^[a-z]+/i.exec(stripped)?.[0].toLowerCase();
  return word !== undefined && !READ_STATEMENT.test(word);
}

/** Pulls an error location out of a database error message: Postgres says
 * `position: 12`, SQLite sometimes `line 2, column 5`. The offset is turned
 * into a line/column against the SQL that was actually run. */
function locateError(message: string, text: string): string | null {
  const lineCol = /line\s+(\d+)(?:\D+col(?:umn)?\s*(\d+))?/i.exec(message);
  if (lineCol) return `Line ${lineCol[1]}${lineCol[2] ? `, column ${lineCol[2]}` : ""}`;
  const offset = /position\D{0,3}(\d+)/i.exec(message);
  if (!offset) return null;
  const index = Math.max(0, Number(offset[1]) - 1);
  const before = text.slice(0, index).split("\n");
  return `Line ${before.length}, column ${(before[before.length - 1]?.length ?? 0) + 1} (position ${offset[1]})`;
}

function cellText(value: unknown): string {
  if (value === null || value === undefined) return "";
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

function toCsv(result: SqlReadResult): string {
  const escape = (value: string) => (/[",\n\r]/.test(value) ? `"${value.replaceAll('"', '""')}"` : value);
  const lines = [result.columns.map(escape).join(",")];
  for (const row of result.rows) lines.push(result.columns.map((column) => escape(cellText(row[column]))).join(","));
  return lines.join("\n");
}

function toJson(result: SqlReadResult): string {
  return JSON.stringify(result.rows, null, 2);
}

function download(filename: string, mime: string, content: string) {
  const url = URL.createObjectURL(new Blob([content], { type: mime }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  URL.revokeObjectURL(url);
}

async function copyText(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`${what} copied`);
  } catch {
    toast.error("Couldn't copy to the clipboard");
  }
}

const IS_MAC = typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform);

/** How one cell renders: `null` as a muted literal so it isn't confused with
 * an empty string, everything else as its plain string form (numbers,
 * booleans-as-0/1, and base64 for BLOB columns all arrive from the server
 * already scalar). */
function CellValue({ value }: { value: unknown }) {
  if (value === null || value === undefined) {
    return <span className="text-muted-foreground/70 italic">NULL</span>;
  }
  return <span className="font-mono text-xs">{cellText(value)}</span>;
}

function relativeTime(at: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

function ResultsSkeleton() {
  return (
    <div aria-busy="true" aria-label="Running query" className="flex flex-col gap-1.5 p-3">
      {Array.from({ length: 5 }, (_, i) => (
        <Skeleton key={i} className="h-row w-full" />
      ))}
    </div>
  );
}

/**
 * A superuser-only ad-hoc SQL prompt against the live database — the same
 * trust tier as the collection-schema editor, not a new one. See
 * `crate::routes::sql_console`'s doc comment for the read/write gate this
 * page's toggle mirrors: the server enforces it independently, so nothing
 * here is a security control, only a guardrail against a fat-fingered
 * write.
 */
export function SqlConsolePage() {
  const [sqlText, setSqlText] = useState("");
  const [writeMode, setWriteMode] = useState(false);
  const [confirmingWrite, setConfirmingWrite] = useState(false);
  const [editorHeight, setEditorHeight] = useState(176);
  const [history, setHistory] = useState<HistoryEntry[]>(() => readList<HistoryEntry>(HISTORY_KEY));
  const [saved, setSaved] = useState<SavedQuery[]>(() => readList<SavedQuery>(SAVED_KEY));
  const [saveName, setSaveName] = useState("");
  const [historyOpen, setHistoryOpen] = useState(false);
  const drag = useRef<{ startY: number; startHeight: number } | null>(null);
  const { resolvedTheme } = useTheme();

  const run = useMutation({
    mutationFn: async (payload: { sql: string; write: boolean }): Promise<RunOutcome> => {
      const started = performance.now();
      const result = await cb.send<SqlResult>("/api/sql", { method: "POST", body: payload });
      return { result, ms: performance.now() - started };
    },
  });

  const outcome = run.data;
  const result = outcome?.result;

  function remember(text: string) {
    setHistory((current) => {
      const next = [{ sql: text, at: Date.now() }, ...current.filter((entry) => entry.sql !== text)].slice(0, HISTORY_LIMIT);
      writeList(HISTORY_KEY, next);
      return next;
    });
  }

  function execute(text: string, write: boolean) {
    if (text.trim().length === 0 || run.isPending) return;
    remember(text);
    run.mutate({ sql: text, write });
  }

  // Table-name (and, where the schema exposes it, column-name) completion
  // for the editor below — built from the same collection list the rest of
  // the dashboard already fetches, so this costs no extra request.
  const { data: collections } = useCollections();
  const schema: SQLNamespace = useMemo(() => {
    const namespace: Record<string, string[]> = {};
    for (const collection of collections ?? []) {
      namespace[collection.name] = (collection.fields ?? [])
        .map((field) => (typeof field.name === "string" ? field.name : null))
        .filter((name): name is string => name !== null);
    }
    return namespace;
  }, [collections]);

  // The keymap extension is built once (it doesn't depend on `sqlText` —
  // the command reads straight off the live editor doc, not React state,
  // so a fast typist never runs a stale query) and reaches `execute`/
  // `writeMode` through a ref kept current after every render, the standard way
  // to give a long-lived CodeMirror extension access to values that change on
  // every keystroke without recreating the extension itself.
  const executeRef = useRef<(text: string) => void>(() => {});
  useEffect(() => {
    executeRef.current = (text) => execute(text, writeMode);
  });

  const extensions = useMemo(
    () => [
      sql({ dialect: SQLite, schema, upperCaseKeywords: true }),
      Prec.highest(
        keymap.of([
          {
            key: "Mod-Enter",
            run: (view) => {
              executeRef.current(view.state.doc.toString());
              return true;
            },
          },
        ]),
      ),
    ],
    [schema],
  );

  function saveCurrent() {
    const name = saveName.trim();
    if (!name || sqlText.trim().length === 0) return;
    const next = [{ name, sql: sqlText }, ...saved.filter((entry) => entry.name !== name)];
    setSaved(next);
    writeList(SAVED_KEY, next);
    setSaveName("");
    toast.success("Query saved", { description: name });
  }

  function deleteSaved(name: string) {
    const next = saved.filter((entry) => entry.name !== name);
    setSaved(next);
    writeList(SAVED_KEY, next);
  }

  function clearHistory() {
    setHistory([]);
    writeList(HISTORY_KEY, []);
  }

  function load(text: string) {
    setSqlText(text);
    setHistoryOpen(false);
  }

  const clampHeight = (value: number) => Math.min(EDITOR_MAX, Math.max(EDITOR_MIN, value));

  const isWriteStatement = looksLikeWrite(sqlText);
  const failure = run.isError ? describeFailure(run.error) : null;
  const failureDetail = failure ? failure.detail || failure.serverMessage || "" : "";
  const failureLocation = failure && run.variables ? locateError(failureDetail, run.variables.sql) : null;
  const shortcut = (
    <KbdGroup aria-label={IS_MAC ? "Command Enter" : "Control Enter"}>
      <Kbd>{IS_MAC ? "⌘" : "Ctrl"}</Kbd>
      <Kbd>↵</Kbd>
    </KbdGroup>
  );

  const item = settingsItemFor("/settings/sql")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <div className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2">
        <AlertTriangle aria-hidden className="mt-0.5 size-3.5 shrink-0 text-warning" />
        <p className="min-w-0 flex-1 text-xs text-pretty">
          Raw SQL runs directly against the database and bypasses every collection rule, hook, and validation this
          project defines. It is also dialect-specific — a statement written for SQLite may not run, or may mean
          something different, against Postgres. There is no undo for a write.
        </p>
      </div>

      <div className="overflow-hidden rounded-lg border border-border bg-card shadow-xs">
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-border px-3 py-2">
          <div className="flex flex-wrap items-center gap-4">
            {!confirmingWrite ? (
              <ToggleSetting
                id="sql-console-write-mode"
                checked={writeMode}
                onChange={(checked) => {
                  if (checked) {
                    setConfirmingWrite(true);
                  } else {
                    setWriteMode(false);
                  }
                }}
                label="Write mode (INSERT / UPDATE / DELETE / DDL)"
              />
            ) : (
              <HoldToConfirm
                onConfirm={() => {
                  setWriteMode(true);
                  setConfirmingWrite(false);
                }}
                onAbort={() => setConfirmingWrite(false)}
                confirmLabel="Write mode enabled"
                className="h-control-sm"
              >
                Hold to confirm — I understand this can modify data
              </HoldToConfirm>
            )}
            {writeMode ? <StatusBadge tone="warning">Writes enabled</StatusBadge> : <StatusBadge tone="neutral">Read-only</StatusBadge>}
          </div>

          <div className="flex items-center gap-2">
            <Popover open={historyOpen} onOpenChange={setHistoryOpen}>
              <PopoverTrigger asChild>
                <Button variant="outline" size="sm">
                  <History />
                  History
                  <span className="font-tabular text-muted-foreground">{history.length + saved.length || ""}</span>
                </Button>
              </PopoverTrigger>
              <PopoverContent align="end" className="w-96 gap-0 p-0">
                <form
                  className="flex items-center gap-2 border-b border-border p-2"
                  onSubmit={(event) => {
                    event.preventDefault();
                    saveCurrent();
                  }}
                >
                  <Input
                    aria-label="Name for the current query"
                    placeholder="Save current query as…"
                    value={saveName}
                    onChange={(event) => setSaveName(event.target.value)}
                    className="h-control-sm"
                  />
                  <Button type="submit" size="sm" variant="outline" disabled={!saveName.trim() || sqlText.trim().length === 0}>
                    <BookmarkPlus />
                    Save
                  </Button>
                </form>
                <div className="max-h-80 overflow-auto">
                  {saved.length === 0 && history.length === 0 ? (
                    <p className="px-3 py-6 text-center text-sm text-muted-foreground">
                      Queries you run show up here. Name one above to keep it.
                    </p>
                  ) : null}
                  {saved.length > 0 ? (
                    <section aria-label="Saved queries">
                      <h3 className="px-3 pt-2 pb-1 text-2xs font-medium tracking-wide text-muted-foreground uppercase">Saved</h3>
                      {saved.map((entry) => (
                        <div key={entry.name} className="group flex items-center gap-1 pr-1.5 hover:bg-accent/50">
                          <button type="button" onClick={() => load(entry.sql)} className="min-w-0 flex-1 px-3 py-1.5 text-left">
                            <span className="block truncate text-sm font-medium">{entry.name}</span>
                            <span className="block truncate font-mono text-2xs text-muted-foreground">{entry.sql.replace(/\s+/g, " ")}</span>
                          </button>
                          <Button variant="ghost" size="icon-xs" aria-label={`Delete saved query ${entry.name}`} onClick={() => deleteSaved(entry.name)}>
                            <X />
                          </Button>
                        </div>
                      ))}
                    </section>
                  ) : null}
                  {history.length > 0 ? (
                    <section aria-label="Recent queries">
                      <div className="flex items-center justify-between px-3 pt-2 pb-1">
                        <h3 className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">Recent</h3>
                        <Button variant="ghost" size="xs" onClick={clearHistory}>
                          <Trash2 />
                          Clear
                        </Button>
                      </div>
                      {history.map((entry) => (
                        <button
                          key={entry.at}
                          type="button"
                          onClick={() => load(entry.sql)}
                          className="block w-full px-3 py-1.5 text-left hover:bg-accent/50"
                        >
                          <span className="block truncate font-mono text-xs">{entry.sql.replace(/\s+/g, " ")}</span>
                          <span className="text-2xs text-muted-foreground">{relativeTime(entry.at)}</span>
                        </button>
                      ))}
                    </section>
                  ) : null}
                </div>
              </PopoverContent>
            </Popover>
            <Button size="sm" onClick={() => execute(sqlText, writeMode)} disabled={run.isPending || sqlText.trim().length === 0}>
              {run.isPending ? <Spinner /> : <Play />}
              Run
              <span className="ml-1 hidden sm:inline-flex">{shortcut}</span>
            </Button>
          </div>
        </div>

        {isWriteStatement && sqlText.trim().length > 0 ? (
          <div
            role="status"
            className={cn(
              "flex items-center gap-2 border-b border-border px-3 py-1.5 text-xs",
              writeMode ? "bg-warning/10 text-foreground" : "bg-info/10 text-foreground",
            )}
          >
            <AlertTriangle aria-hidden className={cn("size-3.5 shrink-0", writeMode ? "text-warning" : "text-info")} />
            {writeMode
              ? "This statement modifies data or schema and cannot be undone."
              : "This looks like a write statement. Read-only mode will reject it — turn on write mode above to run it."}
          </div>
        ) : null}

        <CodeMirror
          value={sqlText}
          onChange={setSqlText}
          theme={resolvedTheme === "light" ? "light" : "dark"}
          extensions={extensions}
          placeholder="SELECT * FROM _collections LIMIT 10"
          height={`${editorHeight}px`}
          basicSetup={{ foldGutter: false, highlightActiveLine: false }}
          className="text-sm [&_.cm-editor]:outline-none [&_.cm-placeholder]:italic"
        />

        <div
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize editor"
          aria-valuenow={editorHeight}
          aria-valuemin={EDITOR_MIN}
          aria-valuemax={EDITOR_MAX}
          tabIndex={0}
          title="Drag to resize"
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            drag.current = { startY: event.clientY, startHeight: editorHeight };
          }}
          onPointerMove={(event) => {
            if (drag.current) setEditorHeight(clampHeight(drag.current.startHeight + event.clientY - drag.current.startY));
          }}
          onPointerUp={() => {
            drag.current = null;
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowUp") setEditorHeight((h) => clampHeight(h - 24));
            else if (event.key === "ArrowDown") setEditorHeight((h) => clampHeight(h + 24));
            else return;
            event.preventDefault();
          }}
          className="group flex h-2.5 cursor-row-resize touch-none items-center justify-center border-y border-border bg-surface-sunken transition-colors duration-fast ease-out-strong hover:bg-accent focus-visible:bg-accent"
        >
          <span className="h-0.5 w-8 rounded-full bg-border-strong transition-colors duration-fast group-hover:bg-primary group-focus-visible:bg-primary" />
        </div>

        <div className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2">
          <h2 className="text-sm font-medium">Results</h2>
          {outcome && !run.isPending && !run.isError ? (
            <>
              <span className="font-tabular inline-flex items-center gap-1 text-xs text-muted-foreground">
                {isWriteResult(outcome.result)
                  ? `${outcome.result.rowsAffected.toLocaleString()} ${outcome.result.rowsAffected === 1 ? "row" : "rows"} affected`
                  : `${outcome.result.rows.length.toLocaleString()} ${outcome.result.rows.length === 1 ? "row" : "rows"}`}
                <span aria-hidden>·</span>
                <Clock aria-hidden className="size-3" />
                {outcome.ms < 1000 ? `${Math.max(1, Math.round(outcome.ms))} ms` : `${(outcome.ms / 1000).toFixed(2)} s`}
              </span>
              {!isWriteResult(outcome.result) && outcome.result.truncated ? <StatusBadge tone="warning">Truncated</StatusBadge> : null}
              {!isWriteResult(outcome.result) && outcome.result.rows.length > 0 ? (
                <div className="ml-auto flex items-center gap-1.5">
                  <ResultMenu label="Copy" icon={<Copy />} result={outcome.result} />
                  <ResultMenu label="Export" icon={<Download />} result={outcome.result} exportFiles />
                </div>
              ) : null}
            </>
          ) : null}
        </div>

        <div className="border-t border-border">
          {run.isPending ? (
            <ResultsSkeleton />
          ) : failure ? (
            <div className="p-3">
              <Alert variant="destructive">
                <AlertTriangle />
                <AlertTitle>{failure.title}</AlertTitle>
                <AlertDescription>
                  {failureDetail ? (
                    <pre className="font-mono text-xs break-words whitespace-pre-wrap">{failureDetail}</pre>
                  ) : null}
                  {failureLocation ? <p className="font-tabular mt-1 text-xs">{failureLocation}</p> : null}
                </AlertDescription>
              </Alert>
            </div>
          ) : result === undefined ? (
            <EmptyState
              compact
              icon={<Play />}
              title="No query run yet"
              description={
                <span className="inline-flex flex-wrap items-center justify-center gap-1.5">
                  Write a statement above, then press Run or {shortcut}.
                </span>
              }
            />
          ) : isWriteResult(result) ? (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">
              Statement ran. <span className="font-tabular text-foreground">{result.rowsAffected.toLocaleString()}</span>{" "}
              {result.rowsAffected === 1 ? "row" : "rows"} affected.
            </p>
          ) : result.rows.length === 0 ? (
            <EmptyState compact title="No rows" description="The query ran but returned nothing." />
          ) : (
            <ResultTable result={result} />
          )}
        </div>
      </div>
    </SettingsPage>
  );
}

function ResultTable({ result }: { result: SqlReadResult }) {
  // A column is numeric when its first non-null value is — right-aligned so
  // the digits line up.
  const numeric = useMemo(
    () =>
      new Set(
        result.columns.filter((column) => {
          const first = result.rows.find((row) => row[column] !== null && row[column] !== undefined);
          return first !== undefined && typeof first[column] === "number";
        }),
      ),
    [result],
  );
  return (
    <>
      <div className="max-h-[60vh] overflow-auto">
        <Table>
          <TableHeader className="sticky top-0 z-sticky bg-surface-sunken/95 backdrop-blur-sm">
            <TableRow className="hover:bg-transparent">
              <TableHead className="font-tabular w-10 text-right text-2xs text-muted-foreground">#</TableHead>
              {result.columns.map((column) => (
                <TableHead key={column} className={cn("font-mono text-xs whitespace-nowrap", numeric.has(column) && "text-right")}>
                  {column}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {result.rows.map((row, i) => (
              // eslint-disable-next-line react/no-array-index-key -- rows have no stable id, this is an ad-hoc result set
              <TableRow key={i} className="h-row hover:bg-accent/40">
                <TableCell className="font-tabular w-10 text-right text-2xs text-muted-foreground">{i + 1}</TableCell>
                {result.columns.map((column) => (
                  <TableCell
                    key={column}
                    title={cellText(row[column])}
                    className={cn("max-w-xs truncate whitespace-nowrap", numeric.has(column) && "font-tabular text-right")}
                  >
                    <CellValue value={row[column]} />
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      {result.truncated ? (
        <p className="border-t border-border px-3 py-2 text-xs text-muted-foreground">
          Showing the first <span className="font-tabular">{result.rows.length.toLocaleString()}</span> rows — the query
          returned more than that. Add a <code className="font-mono">LIMIT</code> or a narrower <code className="font-mono">WHERE</code>.
        </p>
      ) : null}
    </>
  );
}

/** Client-side copy or file export of the rows currently shown, as JSON or
 * CSV — nothing is re-queried. */
function ResultMenu({
  label,
  icon,
  result,
  exportFiles = false,
}: {
  label: string;
  icon: React.ReactNode;
  result: SqlReadResult;
  exportFiles?: boolean;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          {icon}
          {label}
          <ChevronDown className="opacity-60" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel className="font-tabular">{result.rows.length.toLocaleString()} rows</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={() =>
            exportFiles ? download("query-result.json", "application/json", toJson(result)) : void copyText(toJson(result), "JSON")
          }
        >
          {exportFiles ? "JSON file" : "As JSON"}
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => (exportFiles ? download("query-result.csv", "text/csv", toCsv(result)) : void copyText(toCsv(result), "CSV"))}
        >
          {exportFiles ? "CSV file" : "As CSV"}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
