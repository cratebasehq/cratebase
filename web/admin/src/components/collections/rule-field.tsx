import { useId } from "react";
import { AlertCircle, HelpCircle, Lock, LockOpen, ShieldCheck } from "lucide-react";
import { StatusBadge } from "@/components/layout/stat-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

interface RuleFieldProps {
  label: string;
  value: string | null;
  onChange: (value: string | null) => void;
  /** Overrides for the three segmented-control options and their helper
   * text below — the defaults ("Admins"/"Public"/"Custom", API-rule
   * wording) fit a collection API rule; a non-rule reuse of this same
   * `null`/`""`/expression shape (e.g. `_emailTriggers.condition`, where
   * `null` means "always fires" rather than "superusers only") can pass
   * its own wording instead. */
  nullOption?: { label: string; description: string };
  publicOption?: { label: string; description: string };
  customLabel?: string;
  /** One sentence on who this rule governs, e.g. "Who can list and search
   * records." Shown under the label. */
  description?: string;
  /** Name of the field holding a record's owner id. When set, an "Owner"
   * preset (`field = @request.auth.id`) is offered. Only used in the
   * default API-rule mode. */
  ownerField?: string | null;
}

type RuleMode = "admin" | "public" | "auth" | "owner" | "custom";

const AUTH_EXPR = '@request.auth.id != ""';

function ownerExpr(ownerField: string): string {
  return `${ownerField} = @request.auth.id`;
}

function modeOf(value: string | null, ownerField: string | null | undefined): RuleMode {
  if (value === null) return "admin";
  if (value === "") return "public";
  const normalised = value.trim().replace(/'/g, '"');
  if (normalised === AUTH_EXPR) return "auth";
  if (ownerField && normalised === ownerExpr(ownerField)) return "owner";
  return "custom";
}

/** A cheap structural check — unbalanced quotes/parentheses, a dangling
 * operator, an empty body. It is not the real parser (that runs on save
 * server-side) but catches the typos that otherwise only surface as a 400. */
function lintRuleExpression(expr: string): string | null {
  const text = expr.trim();
  if (text.length === 0) return "Enter an expression, or pick Public / Superusers only.";
  let depth = 0;
  let quote: string | null = null;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]!;
    if (quote) {
      if (ch === "\\") i++;
      else if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'") quote = ch;
    else if (ch === "(") depth++;
    else if (ch === ")") {
      depth--;
      if (depth < 0) return "Unmatched closing parenthesis.";
    }
  }
  if (quote) return `Unclosed ${quote === '"' ? "double" : "single"} quote.`;
  if (depth > 0) return "Unclosed parenthesis.";
  if (/(&&|\|\||[=<>~]|!=|!~)\s*$/.test(text)) return "Expression ends with an operator.";
  if (/^\s*(&&|\|\|)/.test(text)) return "Expression starts with an operator.";
  if (!/[=<>~]/.test(text) && !/^(true|false)$/i.test(text)) {
    return "No comparison found — use an operator such as = or ~.";
  }
  return null;
}

const OPERATORS: { op: string; desc: string }[] = [
  { op: "=  !=", desc: "equals / not equals" },
  { op: ">  >=  <  <=", desc: "ordering (numbers, dates, strings)" },
  { op: "~  !~", desc: "contains / doesn't contain (substring match)" },
  { op: "?=  ?!=  ?>  ?>=  ?<  ?<=  ?~  ?!~", desc: "any-of variants — match if any value in a multi-value field (select/relation) satisfies the comparison" },
  { op: "&&  ||", desc: "and / or, with ( ) for grouping" },
];

const EXAMPLES: { expr: string; desc: string }[] = [
  { expr: "@request.auth.id != \"\"", desc: "any authenticated caller" },
  { expr: "owner = @request.auth.id", desc: "only the record's owner" },
  { expr: "status = \"published\" || owner = @request.auth.id", desc: "public once published, otherwise owner-only" },
  { expr: "author.name = \"Ada\"", desc: "dot-notation reaches into a related record" },
  { expr: "tags ?= \"featured\"", desc: "any-of — true if \"featured\" is among a multi-value field's values" },
  { expr: "@request.data.status = \"draft\"", desc: "compare against the incoming request payload (create/update rules)" },
];

/** Compact reference for the filter-expression grammar `crates/filter`
 * parses, so a rule author doesn't have to go spelunking through the Rust
 * source to remember an operator. Kept in sync with
 * `crates/filter/src/{lexer,ast,parser}.rs`. */
function SyntaxHelp() {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="Filter expression syntax help"
          className="text-muted-foreground hover:text-foreground"
        >
          <HelpCircle className="size-3.5" />
        </Button>
      </PopoverTrigger>
      <PopoverContent side="top" align="end" className="w-80 p-3">
        <div className="flex flex-col gap-3">
          <div>
            <p className="mb-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground/70">Operators</p>
            <dl className="flex flex-col gap-1">
              {OPERATORS.map((o) => (
                <div key={o.op} className="flex flex-col">
                  <dt className="font-mono text-xs text-foreground">{o.op}</dt>
                  <dd className="text-2xs leading-snug text-muted-foreground">{o.desc}</dd>
                </div>
              ))}
            </dl>
          </div>
          <div>
            <p className="mb-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground/70">Context values</p>
            <p className="text-2xs leading-snug text-muted-foreground">
              <code className="font-mono text-foreground">@request.auth.*</code> reads the caller's own auth record
              (e.g. <code className="font-mono">@request.auth.id</code>); it's <code className="font-mono">""</code> for
              anonymous callers. <code className="font-mono text-foreground">@request.data.*</code> reads the incoming
              create/update payload.
            </p>
          </div>
          <div>
            <p className="mb-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground/70">Examples</p>
            <ul className="flex flex-col gap-1.5">
              {EXAMPLES.map((e) => (
                <li key={e.expr}>
                  <code className="block break-all font-mono text-xs text-foreground">{e.expr}</code>
                  <span className="text-2xs leading-snug text-muted-foreground">{e.desc}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** One API rule (list/view/create/update/delete). `null` = superusers
 * only, `""` = public, anything else = a filter expression evaluated
 * against the caller/record. In the default mode the rule also gets
 * presets (signed-in, owner-only) and a status chip that says plainly
 * whether it is locked or open. */
export function RuleField({
  label,
  value,
  onChange,
  nullOption,
  publicOption,
  customLabel = "Custom",
  description,
  ownerField,
}: RuleFieldProps) {
  const apiMode = !nullOption && !publicOption;
  const nullOpt = nullOption ?? { label: "Superusers", description: "Locked — only superusers can do this." };
  const publicOpt = publicOption ?? { label: "Public", description: "Open — anyone, including anonymous callers." };
  const mode = modeOf(value, apiMode ? ownerField : null);
  const id = useId();
  const lint = mode === "custom" ? lintRuleExpression(value ?? "") : null;

  function setMode(next: string) {
    if (next === "admin") onChange(null);
    else if (next === "public") onChange("");
    else if (next === "auth") onChange(AUTH_EXPR);
    else if (next === "owner" && ownerField) onChange(ownerExpr(ownerField));
    else onChange(mode === "custom" && value ? value : " ");
  }

  const hint: Record<RuleMode, string> = {
    admin: nullOpt.description,
    public: publicOpt.description,
    auth: "Any signed-in user (any auth collection). Anonymous callers are rejected.",
    owner: `Signed in, and the record's ${ownerField ?? "owner"} field must be the caller's own id.`,
    custom: "Matches when the expression below is true for the caller and the record.",
  };

  return (
    <div className="flex flex-col gap-2 rounded-lg border border-border bg-card px-3 py-2.5">
      <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-1.5">
        <div className="flex min-w-0 flex-col">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-foreground">{label}</span>
            {apiMode ? (
              mode === "admin" ? (
                <StatusBadge tone="neutral">
                  <Lock className="size-3" />
                  Locked
                </StatusBadge>
              ) : mode === "public" ? (
                <StatusBadge tone="warning">
                  <LockOpen className="size-3" />
                  Open to all
                </StatusBadge>
              ) : (
                <StatusBadge tone="info">
                  <ShieldCheck className="size-3" />
                  Rule
                </StatusBadge>
              )
            ) : null}
          </div>
          {description ? <span className="text-xs text-muted-foreground">{description}</span> : null}
        </div>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          spacing={0}
          aria-label={`${label} access`}
          value={mode}
          // A segmented control always has exactly one option picked —
          // Radix reports "" when the pressed item is toggled off.
          onValueChange={(next) => {
            if (next) setMode(next);
          }}
        >
          <ToggleGroupItem value="admin">{nullOpt.label}</ToggleGroupItem>
          {apiMode ? <ToggleGroupItem value="auth">Signed in</ToggleGroupItem> : null}
          {apiMode && ownerField ? <ToggleGroupItem value="owner">Owner</ToggleGroupItem> : null}
          <ToggleGroupItem value="public">{publicOpt.label}</ToggleGroupItem>
          <ToggleGroupItem value="custom">{customLabel}</ToggleGroupItem>
        </ToggleGroup>
      </div>
      {mode === "custom" ? (
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-1.5">
            <Input
              type="text"
              value={value ?? ""}
              onChange={(e) => onChange(e.target.value)}
              placeholder="e.g. owner = @request.auth.id"
              aria-label={`${label} rule expression`}
              aria-invalid={lint ? true : undefined}
              aria-describedby={`${id}-msg`}
              spellCheck={false}
              className="h-control-md min-w-0 flex-1 font-mono text-sm"
            />
            <SyntaxHelp />
          </div>
          <p
            id={`${id}-msg`}
            role={lint ? "alert" : undefined}
            className={lint ? "flex items-center gap-1 text-xs font-medium text-destructive" : "text-xs text-muted-foreground"}
          >
            {lint ? <AlertCircle className="size-3 shrink-0" /> : null}
            {lint ?? hint.custom}
          </p>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">{hint[mode]}</p>
      )}
    </div>
  );
}
