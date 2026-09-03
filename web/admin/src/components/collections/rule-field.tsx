import { SegmentedControl } from "@/components/interior/segmented-control";

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
        <input
          type="text"
          value={value ?? ""}
          onChange={(e) => onChange(e.target.value)}
          placeholder='e.g. owner = @request.auth.id'
          className="h-9 w-full rounded-[9px] border-2 border-border bg-secondary/60 px-2.5 font-mono text-[12.5px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-primary focus:bg-card"
        />
      ) : (
        <p className="text-[11.5px] text-muted-foreground">
          {mode === "admin" ? "Only superusers can do this." : "Anyone, including anonymous callers, can do this."}
        </p>
      )}
    </div>
  );
}
