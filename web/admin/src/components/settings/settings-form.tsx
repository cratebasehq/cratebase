import type { ReactNode } from "react";
import { AlertCircle, Undo2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";

/** A titled block of related settings. */
export function SettingsSection({
  title,
  description,
  children,
  action,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex min-w-0 flex-col">
          <h2 className="text-sm font-medium">{title}</h2>
          {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
        </div>
        {action}
      </div>
      <div className="flex flex-col gap-3">{children}</div>
    </section>
  );
}

/** Label on the left, control on the right, help underneath — the shape a
 * settings screen wants, rather than a stack of full-width inputs. */
export function SettingRow({
  label,
  help,
  htmlFor,
  error,
  children,
}: {
  label: string;
  help?: string;
  htmlFor?: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <div className="grid grid-cols-1 gap-1 sm:grid-cols-[14rem_1fr] sm:gap-3">
      <label htmlFor={htmlFor} className="pt-1.5 text-sm text-foreground">
        {label}
      </label>
      <div className="flex min-w-0 flex-col gap-1">
        {children}
        {error ? (
          <p className="flex items-center gap-1 text-xs text-destructive">
            <AlertCircle className="size-3 shrink-0" />
            {error}
          </p>
        ) : help ? (
          <p className="text-2xs leading-snug text-muted-foreground">{help}</p>
        ) : null}
      </div>
    </div>
  );
}

export function TextSetting({
  id,
  value,
  onChange,
  placeholder,
  type = "text",
  mono = false,
  invalid,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  type?: string;
  mono?: boolean;
  invalid?: boolean;
}) {
  return (
    <Input
      id={id}
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      aria-invalid={invalid || undefined}
      className={cn("h-control-md", mono && "font-mono text-sm")}
    />
  );
}

export function NumberSetting({
  id,
  value,
  onChange,
  min,
  suffix,
  invalid,
}: {
  id: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
  suffix?: string;
  invalid?: boolean;
}) {
  return (
    <div className="flex items-center gap-2">
      <Input
        id={id}
        type="number"
        min={min}
        value={Number.isFinite(value) ? value : 0}
        onChange={(e) => onChange(e.target.value === "" ? 0 : Number(e.target.value))}
        aria-invalid={invalid || undefined}
        className="h-control-md w-32 font-tabular"
      />
      {suffix ? <span className="text-xs text-muted-foreground">{suffix}</span> : null}
    </div>
  );
}

export function ToggleSetting({
  id,
  checked,
  onChange,
  label,
}: {
  id: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
}) {
  return (
    <div className="flex items-center gap-2 pt-1">
      <Switch id={id} checked={checked} onCheckedChange={onChange} />
      <label htmlFor={id} className="cursor-pointer text-sm text-muted-foreground">
        {label}
      </label>
    </div>
  );
}

/**
 * A write-only secret. The server never sends these back, so the field
 * cannot show what is stored — it can only say whether something is, and
 * accept a replacement.
 */
export function SecretSetting({
  id,
  value,
  onChange,
  storedHint,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  storedHint: string;
}) {
  return (
    <div className="flex flex-col gap-1">
      <Input
        id={id}
        type="password"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={storedHint}
        autoComplete="new-password"
        className="h-control-md"
      />
    </div>
  );
}

/** The sticky commit bar every settings page shares. */
export function SettingsSaveBar({
  dirty,
  pending,
  errors,
  onSave,
  onReset,
}: {
  dirty: boolean;
  pending: boolean;
  errors: string[];
  onSave: () => void;
  onReset: () => void;
}) {
  if (!dirty) return null;
  return (
    <div className="sticky bottom-0 -mx-page -mb-page flex flex-wrap items-center gap-3 border-t border-border bg-background/95 px-page py-2.5 backdrop-blur-sm">
      <span className="text-sm text-muted-foreground">Unsaved changes</span>
      {errors.length > 0 ? (
        <span className="flex min-w-0 items-center gap-1.5 text-sm text-destructive">
          <AlertCircle className="size-3.5 shrink-0" />
          <span className="truncate">{errors[0]}</span>
        </span>
      ) : null}
      <div className="flex-1" />
      <Button type="button" variant="ghost" size="sm" className="gap-1.5" onClick={onReset} disabled={pending}>
        <Undo2 className="size-3.5" />
        Discard
      </Button>
      <Button type="button" size="sm" onClick={onSave} disabled={pending || errors.length > 0}>
        {pending ? <Spinner /> : null}
        {pending ? "Saving…" : "Save changes"}
      </Button>
    </div>
  );
}
