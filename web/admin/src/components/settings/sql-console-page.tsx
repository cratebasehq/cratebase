import { useMemo, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTheme } from "next-themes";
import { AlertTriangle, Play } from "lucide-react";
import CodeMirror from "@uiw/react-codemirror";
import { sql, SQLite, type SQLNamespace } from "@codemirror/lang-sql";
import { keymap } from "@codemirror/view";
import { Prec } from "@codemirror/state";
import { cb, describeFailure } from "@/lib/api";
import { useCollections } from "@/hooks/use-collections";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { HoldToConfirm } from "@/components/ui/hold-to-confirm";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ToggleSetting } from "@/components/settings/settings-form";

/** As `POST /api/sql` returns them — see `crate::routes::sql_console`. A
 * `SELECT`/`WITH` answers with `columns`/`rows`/`truncated`, anything else
 * with `rowsAffected`; the two shapes never share a field, so `"rowsAffected"
 * in result` tells them apart without a discriminant on the wire. */
type SqlReadResult = { columns: string[]; rows: Array<Record<string, unknown>>; truncated: boolean };
type SqlWriteResult = { rowsAffected: number };
type SqlResult = SqlReadResult | SqlWriteResult;

function isWriteResult(result: SqlResult): result is SqlWriteResult {
  return "rowsAffected" in result;
}

/** How one cell renders: `null` as a muted literal so it isn't confused with
 * an empty string, everything else as its plain string form (numbers,
 * booleans-as-0/1, and base64 for BLOB columns all arrive from the server
 * already scalar). */
function CellValue({ value }: { value: unknown }) {
  if (value === null || value === undefined) {
    return <span className="text-muted-foreground/60">NULL</span>;
  }
  return <span className="font-mono text-xs">{String(value)}</span>;
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
  const { resolvedTheme } = useTheme();

  const run = useMutation({
    mutationFn: (payload: { sql: string; write: boolean }) =>
      cb.send<SqlResult>("/api/sql", { method: "POST", body: payload }),
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const result = run.data;

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
  // so a fast typist never runs a stale query) and reaches `run`/`writeMode`
  // through refs kept current every render, the standard way to give a
  // long-lived CodeMirror extension access to values that change on every
  // keystroke without recreating the extension itself.
  const runMutateRef = useRef(run.mutate);
  runMutateRef.current = run.mutate;
  const writeModeRef = useRef(writeMode);
  writeModeRef.current = writeMode;
  const isPendingRef = useRef(run.isPending);
  isPendingRef.current = run.isPending;

  const extensions = useMemo(
    () => [
      sql({ dialect: SQLite, schema, upperCaseKeywords: true }),
      Prec.highest(
        keymap.of([
          {
            key: "Mod-Enter",
            run: (view) => {
              const text = view.state.doc.toString();
              if (text.trim().length === 0 || isPendingRef.current) return true;
              runMutateRef.current({ sql: text, write: writeModeRef.current });
              return true;
            },
          },
        ]),
      ),
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps -- refs above intentionally excluded, see comment
    [schema],
  );

  return (
    <div className="flex flex-col gap-page p-page">
      <div className="flex flex-wrap items-start gap-3 rounded-lg border border-warning/40 bg-warning/[0.06] px-3 py-2">
        <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warning" />
        <p className="min-w-0 flex-1 text-sm">
          Raw SQL runs directly against the database and bypasses every collection rule, hook, and validation this
          project defines. It is also dialect-specific — a statement written for SQLite may not run, or may mean
          something different, against Postgres. There is no undo for a write.
        </p>
      </div>

      <div className="flex flex-col gap-3 rounded-lg border border-border bg-card p-3">
        <CodeMirror
          value={sqlText}
          onChange={setSqlText}
          theme={resolvedTheme === "light" ? "light" : "dark"}
          extensions={extensions}
          placeholder="SELECT * FROM _collections LIMIT 10"
          minHeight="9rem"
          basicSetup={{ foldGutter: false, highlightActiveLine: false }}
          className="overflow-hidden rounded-md border border-border/60 text-sm [&_.cm-placeholder]:italic"
        />

        <div className="flex flex-wrap items-center justify-between gap-3">
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
                label="Write mode (allow INSERT / UPDATE / DELETE / DDL)"
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
          </div>

          <div className="flex items-center gap-2.5">
            <span className="hidden text-2xs text-muted-foreground sm:inline">⌘/Ctrl + Enter to run</span>
            <Button
              onClick={() => run.mutate({ sql: sqlText, write: writeMode })}
              disabled={run.isPending || sqlText.trim().length === 0}
              className="gap-1.5"
            >
              {run.isPending ? <Spinner className="size-4" /> : <Play className="size-4" />}
              Run
            </Button>
          </div>
        </div>
      </div>

      <div className="flex flex-col gap-2 rounded-lg border border-border bg-card p-3">
        <h2 className="text-sm font-medium">Results</h2>

        {run.isPending ? (
          <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
            <Spinner className="size-4" />
            Running…
          </div>
        ) : run.isError ? null : result === undefined ? (
          <Empty className="py-6">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Play />
              </EmptyMedia>
              <EmptyTitle>No query run yet</EmptyTitle>
              <EmptyDescription>Write a statement above and press Run.</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : isWriteResult(result) ? (
          <p className="text-sm text-muted-foreground">
            {result.rowsAffected} {result.rowsAffected === 1 ? "row" : "rows"} affected.
          </p>
        ) : result.rows.length === 0 ? (
          <Empty className="py-6">
            <EmptyHeader>
              <EmptyTitle>No rows</EmptyTitle>
              <EmptyDescription>The query ran but returned nothing.</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <div className="flex flex-col gap-2">
            <div className="overflow-auto rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow>
                    {result.columns.map((column) => (
                      <TableHead key={column} className="font-mono text-xs">
                        {column}
                      </TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {result.rows.map((row, i) => (
                    // eslint-disable-next-line react/no-array-index-key -- rows have no stable id, this is an ad-hoc result set
                    <TableRow key={i}>
                      {result.columns.map((column) => (
                        <TableCell key={column}>
                          <CellValue value={row[column]} />
                        </TableCell>
                      ))}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
            {result.truncated ? (
              <p className="text-xs text-muted-foreground">
                Showing the first {result.rows.length} rows — the query returned more than that.
              </p>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
