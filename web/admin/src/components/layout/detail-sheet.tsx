import * as React from "react";
import {
  Check,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  CircleCheck,
  Copy,
  Ellipsis,
  Info,
  TriangleAlert,
  X,
} from "lucide-react";
import { Tabs as TabsPrimitive } from "radix-ui";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetTitle,
} from "@/components/ui/sheet";

/* -------------------------------------------------------------------------- */
/* Shell                                                                      */
/* -------------------------------------------------------------------------- */

export interface DetailSheetProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: React.ReactNode;
  className?: string;
}

/** Right-hand detail drawer: 560px on desktop, full width on mobile. Compose
 * `DetailSheetHeader`, optional `DetailTabs`, then `DetailBody`. */
export function DetailSheet({ open, onOpenChange, children, className }: DetailSheetProps) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        showCloseButton={false}
        // Land focus on the dialog itself, not the first button, so opening a
        // sheet never draws a ring on the copy button.
        onOpenAutoFocus={(e) => {
          e.preventDefault();
          (e.currentTarget as HTMLElement).focus();
        }}
        className={cn(
          "gap-0 p-0 outline-none data-[side=right]:w-full data-[side=right]:sm:max-w-[560px]",
          className,
        )}
      >
        {children}
      </SheetContent>
    </Sheet>
  );
}

/* -------------------------------------------------------------------------- */
/* Header                                                                     */
/* -------------------------------------------------------------------------- */

export interface DetailPager {
  /** 1-based position of the open item. */
  index: number;
  total: number;
  onPrev: () => void;
  onNext: () => void;
  hasPrev: boolean;
  hasNext: boolean;
}

export interface DetailMenuItem {
  label: string;
  onSelect: () => void;
  destructive?: boolean;
  icon?: React.ReactNode;
}

export interface DetailSheetHeaderProps {
  /** Machine id; copied to the clipboard by the copy button. */
  id?: string;
  /** What is displayed instead of `id` (e.g. a short form). */
  idLabel?: string;
  pager?: DetailPager;
  menu?: DetailMenuItem[];
  title: React.ReactNode;
  /** Screen-reader description for the dialog. Defaults to the id/title. */
  description?: string;
  /** Usually a `<StatusBadge>`. */
  status?: React.ReactNode;
  /** Segments joined by dots; use `font-mono` on identifiers. */
  meta?: React.ReactNode[];
  /** Action buttons row (`Button size="sm"`). */
  actions?: React.ReactNode;
}

function isTypingTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  if (t.isContentEditable) return true;
  if (/^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return true;
  // Arrow keys belong to these widgets while they own focus.
  return !!t.closest('[role="menu"],[role="listbox"],[role="tablist"],[role="combobox"],[role="textbox"],[role="slider"]');
}

/** `k`/ArrowUp → previous, `j`/ArrowDown → next, while the header is mounted. */
function usePagerKeys(pager: DetailPager | undefined) {
  const ref = React.useRef(pager);
  React.useEffect(() => {
    ref.current = pager;
  });
  const enabled = !!pager;
  React.useEffect(() => {
    if (!enabled) return;
    const onKey = (e: KeyboardEvent) => {
      const p = ref.current;
      if (!p || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      if (isTypingTarget(e.target)) return;
      const dir = e.key === "k" || e.key === "ArrowUp" ? -1 : e.key === "j" || e.key === "ArrowDown" ? 1 : 0;
      if (!dir) return;
      if (dir < 0 ? !p.hasPrev : !p.hasNext) return;
      e.preventDefault();
      (dir < 0 ? p.onPrev : p.onNext)();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [enabled]);
}

function CopyIdButton({ id }: { id: string }) {
  const [done, setDone] = React.useState(false);
  React.useEffect(() => {
    if (!done) return;
    const t = window.setTimeout(() => setDone(false), 1200);
    return () => window.clearTimeout(t);
  }, [done]);
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      aria-label="Copy ID"
      className="text-muted-foreground"
      onClick={() => {
        navigator.clipboard
          .writeText(id)
          .then(() => {
            setDone(true);
            toast.success("ID copied");
          })
          .catch(() => toast.error("Couldn't copy to the clipboard"));
      }}
    >
      {done ? <Check className="text-success" /> : <Copy />}
    </Button>
  );
}

function Pager({ pager }: { pager: DetailPager }) {
  return (
    <div
      role="group"
      aria-label="Browse items"
      className="inline-flex h-control-sm items-center overflow-hidden rounded-lg border border-border dark:border-input"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        className="rounded-none"
        aria-label="Previous item (K)"
        disabled={!pager.hasPrev}
        onClick={pager.onPrev}
      >
        <ChevronUp />
      </Button>
      <span className="font-tabular border-x border-border px-3 text-xs whitespace-nowrap select-none dark:border-input">
        {pager.index} of {pager.total}
      </span>
      <Button
        variant="ghost"
        size="icon-sm"
        className="rounded-none"
        aria-label="Next item (J)"
        disabled={!pager.hasNext}
        onClick={pager.onNext}
      >
        <ChevronDown />
      </Button>
    </div>
  );
}

function OverflowMenu({ items }: { items: DetailMenuItem[] }) {
  const firstDestructive = items.findIndex((i) => i.destructive);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label="More actions">
          <Ellipsis />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-40">
        {items.map((item, i) => (
          <React.Fragment key={item.label}>
            {i > 0 && i === firstDestructive ? <DropdownMenuSeparator /> : null}
            <DropdownMenuItem
              variant={item.destructive ? "destructive" : "default"}
              onSelect={item.onSelect}
            >
              {item.icon}
              {item.label}
            </DropdownMenuItem>
          </React.Fragment>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function DetailSheetHeader({
  id,
  idLabel,
  pager,
  menu,
  title,
  description,
  status,
  meta,
  actions,
}: DetailSheetHeaderProps) {
  usePagerKeys(pager);
  const segments = (meta ?? []).filter((m) => m !== null && m !== undefined && m !== false);
  return (
    <header data-slot="detail-header" className="flex shrink-0 flex-col gap-3 border-b border-border px-5 pt-3.5 pb-4">
      <div className="flex h-control-sm items-center gap-1">
        {id ? (
          <>
            <span className="font-mono text-xs text-muted-foreground">
              {idLabel ?? id}
            </span>
            <CopyIdButton id={id} />
          </>
        ) : null}
        <div className="ml-auto flex items-center gap-1.5">
          {pager ? <Pager pager={pager} /> : null}
          {menu && menu.length > 0 ? <OverflowMenu items={menu} /> : null}
          <SheetClose asChild>
            <Button variant="ghost" size="icon-sm" aria-label="Close">
              <X />
            </Button>
          </SheetClose>
        </div>
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
        <SheetTitle className="min-w-0 text-xl font-medium tracking-tight break-words text-foreground">
          {title}
        </SheetTitle>
        {status}
      </div>
      <SheetDescription className="sr-only">
        {description ?? (id ? `Details for ${idLabel ?? id}` : "Details")}
      </SheetDescription>

      {segments.length > 0 ? (
        <div className="-mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
          {segments.map((seg, i) => (
            <React.Fragment key={i}>
              {i > 0 ? (
                <span aria-hidden className="size-[3px] shrink-0 rounded-full bg-border-strong" />
              ) : null}
              <span className="inline-flex min-w-0 items-center gap-1.5 [&_svg]:size-3.5 [&_svg]:shrink-0">
                {seg}
              </span>
            </React.Fragment>
          ))}
        </div>
      ) : null}

      {actions ? <div className="flex flex-wrap items-center gap-2 pt-0.5">{actions}</div> : null}
    </header>
  );
}

/* -------------------------------------------------------------------------- */
/* Tabs                                                                       */
/* -------------------------------------------------------------------------- */

export interface DetailTabItem {
  value: string;
  label: React.ReactNode;
  /** Small count pill after the label. */
  count?: number;
  disabled?: boolean;
}

export interface DetailTabsProps {
  tabs: DetailTabItem[];
  /** Controlled value. Omit for uncontrolled. */
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
  /** `DetailTabPanel`s. */
  children?: React.ReactNode;
  className?: string;
}

/** Underlined tab bar (full-bleed hairlines) plus the panels it controls. */
export function DetailTabs({
  tabs,
  value,
  defaultValue,
  onValueChange,
  children,
  className,
}: DetailTabsProps) {
  const rootProps =
    value !== undefined
      ? { value, onValueChange }
      : { defaultValue: defaultValue ?? tabs[0]?.value, onValueChange };
  return (
    <TabsPrimitive.Root
      data-slot="detail-tabs"
      className={cn("flex min-h-0 flex-1 flex-col", className)}
      {...rootProps}
    >
      <TabsPrimitive.List
        aria-label="Sections"
        className="flex h-10 shrink-0 items-stretch gap-5 overflow-x-auto border-b border-border px-5 [scrollbar-width:none]"
      >
        {tabs.map((t) => (
          <TabsPrimitive.Trigger
            key={t.value}
            value={t.value}
            disabled={t.disabled}
            className={cn(
              "relative inline-flex shrink-0 items-center gap-1.5 text-sm font-medium whitespace-nowrap text-muted-foreground outline-none transition-colors hover:text-foreground disabled:pointer-events-none disabled:opacity-50",
              "focus-visible:rounded-sm focus-visible:ring-3 focus-visible:ring-ring/40",
              "data-[state=active]:text-foreground",
              "after:absolute after:inset-x-0 after:-bottom-px after:h-0.5 after:scale-x-50 after:rounded-full after:bg-foreground after:opacity-0 after:transition-[opacity,transform] after:duration-base after:ease-out-strong data-[state=active]:after:scale-x-100 data-[state=active]:after:opacity-100",
            )}
          >
            {t.label}
            {t.count !== undefined ? (
              <span className="font-tabular inline-flex h-4.5 min-w-5 items-center justify-center rounded-full border border-border px-1.5 text-2xs font-medium text-muted-foreground">
                {t.count}
              </span>
            ) : null}
          </TabsPrimitive.Trigger>
        ))}
      </TabsPrimitive.List>
      {children}
    </TabsPrimitive.Root>
  );
}

/** Panel for one tab; place `DetailBody` (or any content) inside. */
export function DetailTabPanel({
  value,
  className,
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Content>) {
  return (
    <TabsPrimitive.Content
      data-slot="detail-tab-panel"
      value={value}
      className={cn("flex min-h-0 flex-1 flex-col outline-none data-[state=inactive]:hidden", className)}
      {...props}
    />
  );
}

/* -------------------------------------------------------------------------- */
/* Body                                                                       */
/* -------------------------------------------------------------------------- */

/** The scroll container. Sections and callouts stack inside it. */
export function DetailBody({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="detail-body"
      className={cn("flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain px-5 py-4 [&>[data-slot=detail-callout]+[data-slot=detail-section]]:border-t-0", className)}
      {...props}
    />
  );
}

/* -------------------------------------------------------------------------- */
/* Callout                                                                    */
/* -------------------------------------------------------------------------- */

export type DetailCalloutTone = "danger" | "warning" | "info" | "success";

const CALLOUT: Record<DetailCalloutTone, { box: string; icon: string; Icon: React.ComponentType<{ className?: string }> }> = {
  danger: { box: "border-destructive/40 bg-destructive/6", icon: "text-destructive", Icon: CircleAlert },
  warning: { box: "border-warning/45 bg-warning/8", icon: "text-warning", Icon: TriangleAlert },
  info: { box: "border-info/40 bg-info/6", icon: "text-info", Icon: Info },
  success: { box: "border-success/40 bg-success/6", icon: "text-success", Icon: CircleCheck },
};

export interface DetailCalloutProps {
  tone?: DetailCalloutTone;
  /** Override the tone's default icon. */
  icon?: React.ReactNode;
  /** Mono headline, e.g. `Exit 137`. */
  title: React.ReactNode;
  children?: React.ReactNode;
  /** Small outline button, e.g. "View log". */
  action?: React.ReactNode;
  className?: string;
}

export function DetailCallout({
  tone = "danger",
  icon,
  title,
  children,
  action,
  className,
}: DetailCalloutProps) {
  const t = CALLOUT[tone];
  return (
    <div
      role={tone === "danger" || tone === "warning" ? "alert" : "status"}
      data-slot="detail-callout"
      className={cn("mb-3 flex items-start gap-2.5 rounded-lg border px-3 py-2.5", t.box, className)}
    >
      <span className={cn("mt-px flex h-5 shrink-0 items-center [&_svg]:size-4", t.icon)}>
        {icon ?? <t.Icon />}
      </span>
      <div className="min-w-0 flex-1">
        <div className="font-mono text-sm leading-5 font-semibold text-foreground">{title}</div>
        {children ? <div className="mt-0.5 text-sm text-muted-foreground">{children}</div> : null}
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Sections                                                                   */
/* -------------------------------------------------------------------------- */

export interface DetailSectionProps {
  title: React.ReactNode;
  defaultOpen?: boolean;
  children: React.ReactNode;
  className?: string;
}

/** Collapsible group with a hairline above every section after the first.
 * Open state is local and not remembered. */
export function DetailSection({ title, defaultOpen = true, children, className }: DetailSectionProps) {
  return (
    <Collapsible
      defaultOpen={defaultOpen}
      data-slot="detail-section"
      className={cn("group/section border-t border-border first:border-t-0", className)}
    >
      <h3 className="m-0">
        <CollapsibleTrigger className="group/trigger flex h-control-lg w-full items-center justify-between rounded-md text-left text-xs font-medium tracking-wide text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/40">
          {title}
          <ChevronDown
            aria-hidden
            className="size-4 transition-transform duration-base ease-out-strong group-data-[state=open]/trigger:rotate-180"
          />
        </CollapsibleTrigger>
      </h3>
      <CollapsibleContent className="overflow-hidden data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
        <div className="pt-1 pb-4">{children}</div>
      </CollapsibleContent>
    </Collapsible>
  );
}

const GRID_COLS: Record<number, string> = {
  1: "grid-cols-1",
  2: "grid-cols-2",
  3: "grid-cols-2 sm:grid-cols-3",
  4: "grid-cols-2 sm:grid-cols-4",
  5: "grid-cols-3 sm:grid-cols-5",
  6: "grid-cols-3 sm:grid-cols-6",
};

/** N-column label-over-value grid. */
export function DetailGrid({
  cols = 3,
  className,
  ...props
}: React.ComponentProps<"dl"> & { cols?: 1 | 2 | 3 | 4 | 5 | 6 }) {
  return (
    <dl
      data-slot="detail-grid"
      className={cn("m-0 grid gap-x-3 gap-y-3", GRID_COLS[cols], className)}
      {...props}
    />
  );
}

export function DetailGridItem({
  label,
  children,
  className,
  mono = true,
}: {
  label: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  /** Mono value (default); pass false for prose. */
  mono?: boolean;
}) {
  return (
    <div className={cn("min-w-0", className)}>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className={cn("m-0 mt-1 text-sm break-words text-foreground", mono && "font-mono")}>{children}</dd>
    </div>
  );
}

/** Key/value rows with a fixed label column. */
export function DetailList({ className, ...props }: React.ComponentProps<"dl">) {
  return <dl data-slot="detail-list" className={cn("m-0 flex flex-col", className)} {...props} />;
}

export function DetailRow({
  label,
  hint,
  children,
  className,
}: {
  label: React.ReactNode;
  /** Secondary muted text inline after the value. */
  hint?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      data-slot="detail-row"
      className={cn("grid min-h-7 grid-cols-[110px_minmax(0,1fr)] items-center gap-x-3 py-1", className)}
    >
      <dt className="text-sm text-muted-foreground">{label}</dt>
      <dd className="m-0 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-sm text-foreground">
        {children}
        {hint ? <span className="text-muted-foreground">{hint}</span> : null}
      </dd>
    </div>
  );
}

/** Small outlined tag (environment, scope, …). */
export function DetailChip({ className, ...props }: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="detail-chip"
      className={cn(
        "inline-flex h-5 items-center gap-1 rounded-md border border-border bg-background px-1.5 text-xs font-medium whitespace-nowrap text-foreground dark:border-input dark:bg-input/30 [&_svg]:size-3",
        className,
      )}
      {...props}
    />
  );
}

/** List of `DetailListItem`s. */
export function DetailTimeline({ className, ...props }: React.ComponentProps<"ul">) {
  return <ul data-slot="detail-timeline" className={cn("m-0 flex list-none flex-col p-0", className)} {...props} />;
}

/** Value on the left, muted meta (relative time, duration) on the right. */
export function DetailListItem({
  meta,
  children,
  className,
}: {
  meta?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <li
      data-slot="detail-list-item"
      className={cn("flex min-h-6 items-center justify-between gap-3 py-[3px] text-sm", className)}
    >
      <span className="font-tabular min-w-0 truncate text-foreground">{children}</span>
      {meta ? <span className="font-tabular shrink-0 text-xs text-muted-foreground">{meta}</span> : null}
    </li>
  );
}

/** Quiet empty state for a section or tab. */
export function DetailEmpty({
  icon,
  title,
  description,
  action,
  className,
}: {
  icon?: React.ReactNode;
  title: React.ReactNode;
  description?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      data-slot="detail-empty"
      className={cn("flex flex-col items-center gap-1 px-4 py-8 text-center", className)}
    >
      {icon ? <span className="mb-1 text-muted-foreground [&_svg]:size-5">{icon}</span> : null}
      <p className="m-0 text-sm font-medium text-foreground">{title}</p>
      {description ? <p className="m-0 max-w-xs text-sm text-muted-foreground">{description}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}
