import { HelpCircle } from "lucide-react";
import { SegmentedControl } from "@/components/interior/segmented-control";
import { Popover } from "@/components/interior/popover";

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
    <Popover
      label="Filter expression syntax help"
      side="top"
      align="end"
      triggerClassName="!h-6 !w-6 !p-0 !justify-center !border-0 !bg-transparent !text-muted-foreground hover:!text-foreground"
      className="!w-[320px] !p-3"
      trigger={<HelpCircle className="size-3.5" />}
    >
      <div className="flex flex-col gap-3">
        <div>
          <p className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-muted-foreground/70">Operators</p>
          <dl className="flex flex-col gap-1">
            {OPERATORS.map((o) => (
              <div key={o.op} className="flex flex-col">
                <dt className="font-mono text-[11.5px] text-foreground">{o.op}</dt>
                <dd className="text-[10.5px] leading-snug text-muted-foreground">{o.desc}</dd>
              </div>
            ))}
          </dl>
        </div>
        <div>
          <p className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-muted-foreground/70">Context values</p>
          <p className="text-[10.5px] leading-snug text-muted-foreground">
            <code className="font-mono text-foreground">@request.auth.*</code> reads the caller's own auth record
            (e.g. <code className="font-mono">@request.auth.id</code>); it's <code className="font-mono">""</code> for
            anonymous callers. <code className="font-mono text-foreground">@request.data.*</code> reads the incoming
            create/update payload.
          </p>
        </div>
        <div>
          <p className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-muted-foreground/70">Examples</p>
          <ul className="flex flex-col gap-1.5">
            {EXAMPLES.map((e) => (
              <li key={e.expr}>
                <code className="block break-all font-mono text-[11px] text-foreground">{e.expr}</code>
                <span className="text-[10.5px] leading-snug text-muted-foreground">{e.desc}</span>
              </li>
            ))}
          </ul>
        </div>
      </div>
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
        <span className="text-[13px] font-medium text-foreground">{label}</span>
        <SegmentedControl
          label={`${label} access`}
          value={mode}
          onValueChange={setMode}
          options={[
            { value: "admin", label: "Admins" },
            { value: "public", label: "Public" },
            { value: "custom", label: "Custom" },
          ]}
        />
      </div>
      {mode === "custom" ? (
        <div className="flex items-center gap-1.5">
          <input
            type="text"
            value={value ?? ""}
            onChange={(e) => onChange(e.target.value)}
            placeholder='e.g. owner = @request.auth.id'
            className="h-9 w-full min-w-0 flex-1 rounded-[9px] border-2 border-border bg-secondary/60 px-2.5 font-mono text-[12.5px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-primary focus:bg-card"
          />
          <SyntaxHelp />
        </div>
      ) : (
        <p className="text-[11.5px] text-muted-foreground">
          {mode === "admin" ? "Only superusers can do this." : "Anyone, including anonymous callers, can do this."}
        </p>
      )}
    </div>
  );
}
