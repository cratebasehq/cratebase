import { useMemo, useRef, useState } from "react";
import { CircleAlert, CircleHelp, CornerDownLeft, Filter, Search, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Kbd } from "@/components/ui/kbd";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import type { FieldSchema } from "@/lib/field-types";

/**
 * The filter language, as the server implements it. This is a reference,
 * not a query builder: the expression language is the API surface people
 * write against in rules and in SDK calls, so the dashboard teaches it
 * rather than hiding it behind clickable clauses that can't express half
 * of it anyway.
 */
const OPERATORS: { op: string; means: string }[] = [
  { op: "=", means: "equals (case-sensitive)" },
  { op: "!=", means: "not equals" },
  { op: ">  >=  <  <=", means: "compares numbers, dates and text" },
  { op: "~", means: "contains, case-insensitive" },
  { op: "!~", means: "does not contain" },
  { op: "?=  ?!=  ?~  ?>  ?>=  ?<  ?<=", means: "any-of: true when ANY value on a multi-valued side matches" },
  { op: "&&  ||  ( )", means: "and, or, grouping" },
];

const MODIFIERS: { token: string; means: string }[] = [
  { token: ":isset", means: "was the key present in the request body at all" },
  { token: ":length", means: "element count of a multi-value or back-relation field" },
  { token: ":each", means: "apply the operator to every element of a multi-value field" },
  { token: ":lower", means: "lowercase both sides before comparing" },
];

const MACROS: { token: string; means: string }[] = [
  { token: "@now", means: "this instant, UTC" },
  { token: "@yesterday  @tomorrow", means: "same time, ±1 day" },
  { token: "@todayStart  @todayEnd", means: "midnight boundaries of today" },
  { token: "@monthStart  @monthEnd", means: "boundaries of the current month" },
  { token: "@yearStart  @yearEnd", means: "boundaries of the current year" },
  { token: "@second @minute @hour @day @month @weekday @year", means: "the current value of that unit, as a number" },
];

const CONTEXT: { token: string; means: string }[] = [
  { token: "@request.auth.*", means: "the authenticated record — id, email, any field, dot-walked through relations" },
  { token: "@request.body.*", means: "a field of the incoming create/update payload" },
  { token: "@request.query.*  @request.headers.*", means: "query string and header values" },
  { token: "@request.method  @request.context", means: "the verb, and where the call came from" },
  { token: "@collection.name.field", means: "join against another collection" },
  { token: "posts_via_author.field", means: "back-relation: rows that point at this one" },
];

function exampleFilters(fields: FieldSchema[]): string[] {
  const text = fields.find((f) => f.type === "text");
  const bool = fields.find((f) => f.type === "bool");
  const number = fields.find((f) => f.type === "number");
  const select = fields.find((f) => f.type === "select");
  const relation = fields.find((f) => f.type === "relation");
  const out: string[] = [];
  if (text) out.push(`${text.name} ~ "hello"`);
  if (bool) out.push(`${bool.name} = true`);
  if (number) out.push(`${number.name} >= 100 && created > @todayStart`);
  if (select) out.push(`${select.name}:each ?= "value"`);
  if (relation) out.push(`${relation.name} != ""`);
  out.push("created >= @monthStart && created <= @now");
  out.push('comments_via_post.id:length > 0');
  return out.slice(0, 5);
}

/** Field types whose values a person would expect a plain word to search. */
const SEARCHABLE_TYPES = new Set(["text", "editor", "email", "url"]);

function searchableFields(fields: FieldSchema[]): string[] {
  return fields.filter((f) => SEARCHABLE_TYPES.has(f.type)).map((f) => f.name);
}

/**
 * Whether the expression is a bare word rather than a filter.
 *
 * The grammar needs a comparison, so typing `greta` is a syntax error and
 * the server answers with its generic "Something went wrong" — which tells
 * the person nothing about what they did. Detecting it here lets us do what
 * they meant instead of round-tripping to a dead end.
 */
export function looksLikeBareTerm(expr: string): boolean {
  const trimmed = expr.trim();
  if (trimmed.length === 0) return false;
  return !/[=<>~]|&&|\|\|/.test(trimmed);
}

/** `title ~ "x" || body ~ "x"` — the expression a bare word stands in for. */
export function searchExpression(term: string, fields: string[]): string {
  const escaped = term.trim().replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  return fields.map((name) => `${name} ~ "${escaped}"`).join(" || ");
}

interface FilterBarProps {
  /** The filter currently applied to the query, verbatim. */
  value: string;
  onApply: (next: string) => void;
  /** The server's own message from a rejected filter, if the last query 400'd. */
  error?: string | null;
  collectionName: string;
  fields: FieldSchema[];
  className?: string;
}

/**
 * A monospace expression input for the PocketBase filter language, with the
 * grammar one keystroke away and the server's own rejection shown inline.
 *
 * The expression is applied on Enter (not per keystroke) because a
 * half-typed expression is a syntax error, and every syntax error is a
 * round trip.
 */
export function FilterBar({ value, onApply, error, collectionName, fields, className }: FilterBarProps) {
  const [draft, setDraft] = useState(value);
  const [appliedValue, setAppliedValue] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);

  // The applied filter is the source of truth; the input is a draft on top
  // of it. When the URL changes underneath (a cleared filter, a back
  // navigation) the draft follows — adjusted during render rather than in
  // an effect, so there is no frame showing the stale expression.
  if (appliedValue !== value) {
    setAppliedValue(value);
    setDraft(value);
  }

  const dirty = draft !== value;

  // A bare word is not a filter, but it is what people type. Rather than
  // let it 400, expand it into a search across this collection's text
  // fields — and put that expression in the box afterwards, so the syntax
  // is learned rather than hidden.
  const searchable = useMemo(() => searchableFields(fields), [fields]);
  const bareTerm = looksLikeBareTerm(draft) ? draft.trim() : null;
  const canSearch = bareTerm !== null && searchable.length > 0;

  function apply(expression: string) {
    const trimmed = expression.trim();
    if (looksLikeBareTerm(trimmed) && searchable.length > 0) {
      onApply(searchExpression(trimmed, searchable));
      return;
    }
    onApply(trimmed);
  }

  function insert(snippet: string) {
    setDraft((prev) => (prev.trim().length === 0 ? snippet : `${prev.trim()} && ${snippet}`));
    inputRef.current?.focus();
  }

  return (
    <div className={cn("flex min-w-0 flex-col gap-1", className)}>
      <div
        className={cn(
          "flex h-control-sm min-w-0 items-center gap-1.5 rounded-md border border-input bg-background pr-1 pl-2 transition-colors focus-within:border-ring",
          error && "border-destructive focus-within:border-destructive",
        )}
      >
        <Filter className={cn("size-3.5 shrink-0", error ? "text-destructive" : "text-muted-foreground")} />
        <input
          ref={inputRef}
          value={draft}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          aria-label={`Filter ${collectionName}`}
          aria-invalid={error ? true : undefined}
          placeholder={`filter — e.g. created > @todayStart && published = true`}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              apply(draft);
            }
            if (event.key === "Escape") {
              event.preventDefault();
              if (dirty) setDraft(value);
              else inputRef.current?.blur();
            }
          }}
          className="min-w-0 flex-1 bg-transparent font-mono text-sm text-foreground outline-none placeholder:font-mono placeholder:text-muted-foreground/70"
        />

        {dirty ? (
          <button
            type="button"
            onClick={() => apply(draft)}
            className="flex h-control-xs shrink-0 items-center gap-1 rounded px-1.5 text-2xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            Apply
            <CornerDownLeft className="size-3" />
          </button>
        ) : null}

        {value.length > 0 && !dirty ? (
          <button
            type="button"
            aria-label="Clear filter"
            onClick={() => onApply("")}
            className="grid size-control-xs shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <X className="size-3.5" />
          </button>
        ) : null}

        <Popover>
          <PopoverTrigger asChild>
            <button
              type="button"
              aria-label="Filter syntax reference"
              className="grid size-control-xs shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            >
              <CircleHelp className="size-3.5" />
            </button>
          </PopoverTrigger>
          <PopoverContent align="end" side="bottom" className="w-[min(30rem,calc(100vw-2rem))] gap-0 p-0">
            <div className="flex items-center justify-between border-b border-border px-3 py-2">
              <span className="text-xs font-medium">Filter syntax</span>
              <span className="text-2xs text-muted-foreground">
                <Kbd>Enter</Kbd> applies
              </span>
            </div>
            <div className="max-h-96 overflow-y-auto overscroll-contain">
              <div className="flex flex-col gap-3 p-3">
                <HelpSection title="Examples — click to insert">
                  <div className="flex flex-col gap-1">
                    {exampleFilters(fields).map((example) => (
                      <button
                        key={example}
                        type="button"
                        onClick={() => insert(example)}
                        className="truncate rounded border border-border bg-secondary/40 px-2 py-1 text-left font-mono text-2xs text-foreground transition-colors hover:border-border-strong hover:bg-secondary"
                      >
                        {example}
                      </button>
                    ))}
                  </div>
                </HelpSection>
                <HelpSection title="Operators">
                  <HelpTable rows={OPERATORS.map((o) => [o.op, o.means])} />
                </HelpSection>
                <HelpSection title="Modifiers">
                  <HelpTable rows={MODIFIERS.map((m) => [m.token, m.means])} />
                </HelpSection>
                <HelpSection title="Date macros">
                  <HelpTable rows={MACROS.map((m) => [m.token, m.means])} />
                </HelpSection>
                <HelpSection title="Request &amp; joins">
                  <HelpTable rows={CONTEXT.map((c) => [c.token, c.means])} />
                </HelpSection>
              </div>
            </div>
          </PopoverContent>
        </Popover>
      </div>

      {canSearch ? (
        <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
          <Search className="mt-px size-3.5 shrink-0" />
          <span className="min-w-0 break-words">
            <Kbd>Enter</Kbd> searches{" "}
            <span className="font-mono text-foreground">{searchable.slice(0, 3).join(", ")}</span>
            {searchable.length > 3 ? ` and ${searchable.length - 3} more` : ""}. For an exact
            match write{" "}
            <span className="font-mono text-foreground">{`${searchable[0]} = "${bareTerm}"`}</span>.
          </span>
        </p>
      ) : bareTerm !== null ? (
        <p className="flex items-start gap-1.5 text-xs text-destructive">
          <CircleAlert className="mt-px size-3.5 shrink-0" />
          <span className="min-w-0 break-words">
            A filter needs a comparison, and {collectionName} has no text field to search — try{" "}
            <span className="font-mono">id = "{bareTerm}"</span>.
          </span>
        </p>
      ) : error ? (
        <p className="flex items-start gap-1.5 text-xs text-destructive">
          <CircleAlert className="mt-px size-3.5 shrink-0" />
          <span className="min-w-0 break-words">{error}</span>
        </p>
      ) : null}
    </div>
  );
}

function HelpSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h4 className="text-2xs font-semibold uppercase tracking-wider text-muted-foreground/70">{title}</h4>
      {children}
    </section>
  );
}

function HelpTable({ rows }: { rows: [string, string][] }) {
  return (
    <dl className="flex flex-col gap-1">
      {rows.map(([token, means]) => (
        <div key={token} className="grid grid-cols-[minmax(0,11rem)_1fr] items-baseline gap-2">
          <dt className="truncate font-mono text-2xs text-foreground" title={token}>
            {token}
          </dt>
          <dd className="text-2xs leading-snug text-muted-foreground">{means}</dd>
        </div>
      ))}
    </dl>
  );
}
