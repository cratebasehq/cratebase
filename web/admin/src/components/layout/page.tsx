import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * The frame every screen sits in. The app shell gives screens a fixed-height,
 * overflow-hidden slot; `PageShell` is what scrolls inside it, so a screen
 * never has to know about the top bar, the sidebar or the safe gutters.
 *
 *   <PageShell>
 *     <PageHeader title="Webhooks" description="…" actions={<Button>New</Button>} />
 *     <PageSection title="Endpoints">…</PageSection>
 *   </PageShell>
 *
 * `width`: `"full"` for grids and tables (default), `"wide"` for dashboards
 * and forms with side-by-side panels, `"narrow"` for single-column settings.
 * Pass `scroll={false}` when the screen manages its own scroll regions (a
 * records grid with a sticky header and footer).
 */
export function PageShell({
  children,
  width = "full",
  scroll = true,
  className,
}: {
  children: ReactNode;
  width?: "narrow" | "wide" | "full";
  scroll?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex min-h-0 w-full flex-1 flex-col gap-section p-3 sm:p-page",
        scroll ? "overflow-y-auto" : "overflow-hidden",
        width !== "full" && "mx-auto",
        width === "narrow" && "max-w-3xl",
        width === "wide" && "max-w-6xl",
        className,
      )}
    >
      {children}
    </div>
  );
}

/**
 * Title row of a screen: icon + title, one line of description, an actions
 * slot on the right, and an optional `tabs`/`meta` row underneath. Actions
 * wrap under the title on narrow screens rather than squeezing it.
 *
 * Titles are sentence-case nouns ("Request logs"), never verbs. Put the one
 * primary action last in `actions`; secondary actions come before it.
 */
export function PageHeader({
  title,
  description,
  icon,
  actions,
  meta,
  mono = false,
  className,
}: {
  title: ReactNode;
  description?: ReactNode;
  icon?: ReactNode;
  actions?: ReactNode;
  /** Small inline facts under the description (counts, status, origin). */
  meta?: ReactNode;
  /** Render the title in the mono face — for identifiers such as a collection name. */
  mono?: boolean;
  className?: string;
}) {
  return (
    <header className={cn("flex flex-wrap items-start justify-between gap-x-4 gap-y-3", className)}>
      <div className="flex min-w-0 flex-1 items-start gap-3">
        {icon ? (
          <span
            aria-hidden
            className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-card text-muted-foreground shadow-xs [&_svg]:size-4"
          >
            {icon}
          </span>
        ) : null}
        <div className="flex min-w-0 flex-col gap-0.5">
          <h1 className={cn("truncate text-xl font-semibold tracking-tight", mono && "font-mono")}>{title}</h1>
          {description ? (
            <p className="max-w-measure text-sm text-pretty text-muted-foreground">{description}</p>
          ) : null}
          {meta ? <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">{meta}</div> : null}
        </div>
      </div>
      {actions ? <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
    </header>
  );
}

/**
 * A titled block inside a page. `bordered` wraps the content in a card —
 * use it for panels that hold a form or a chart, leave it off for tables and
 * lists that bring their own border.
 */
export function PageSection({
  title,
  description,
  actions,
  bordered = false,
  children,
  className,
}: {
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  bordered?: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("flex min-w-0 flex-col gap-2", className)}>
      {title || actions ? (
        <div className="flex items-end justify-between gap-2">
          <div className="flex min-w-0 flex-col">
            {title ? <h2 className="text-sm font-medium">{title}</h2> : null}
            {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
          </div>
          {actions ? <div className="flex shrink-0 items-center gap-1.5">{actions}</div> : null}
        </div>
      ) : null}
      {bordered ? <div className="rounded-lg border border-border bg-card p-3 shadow-xs">{children}</div> : children}
    </section>
  );
}
