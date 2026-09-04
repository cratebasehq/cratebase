import { HelpCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

interface RuleFieldProps {
  label: string;
  value: string | null;
  onChange: (value: string | null) => void;
}

type RuleMode = "admin" | "public" | "custom";

function modeOf(value: string | null): RuleMode {
  if (value === null) return "admin";
  if (value === "") return "public";
  return "custom";
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
 * against the caller/record. */
export function RuleField({ label, value, onChange }: RuleFieldProps) {
  const mode = modeOf(value);

  function setMode(next: string) {
    if (next === "admin") onChange(null);
    else if (next === "public") onChange("");
    else onChange(value && value.length > 0 ? value : " ");
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium text-foreground">{label}</span>
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
          <ToggleGroupItem value="admin">Admins</ToggleGroupItem>
          <ToggleGroupItem value="public">Public</ToggleGroupItem>
          <ToggleGroupItem value="custom">Custom</ToggleGroupItem>
        </ToggleGroup>
      </div>
      {mode === "custom" ? (
        <div className="flex items-center gap-1.5">
          <Input
            type="text"
            value={value ?? ""}
            onChange={(e) => onChange(e.target.value)}
            placeholder="e.g. owner = @request.auth.id"
            aria-label={`${label} rule expression`}
            className="h-control-md min-w-0 flex-1 font-mono text-sm"
          />
          <SyntaxHelp />
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          {mode === "admin" ? "Only superusers can do this." : "Anyone, including anonymous callers, can do this."}
        </p>
      )}
    </div>
  );
}
