# Cratebase admin — design system

Read this before building or restyling a screen. Everything lives in
`src/index.css` (tokens) and `src/components/layout/*` (page primitives).

## Identity

Warm ink and paper, **one** accent: crate orange (the colour of the logo mark
and favicon). Neutrals lean a hair warm. Dense, hairline-bordered, developer-
tool density: 13px body, 28–32px controls, 32px table rows.

The accent is spent on three things only: the primary action, the focus ring,
and "you are here" (active nav item, selected tab). Everything else saturated
carries meaning — red failure, green success, yellow warning, blue info.

Fonts: **Geist** (sans) and **Geist Mono** (ids, timestamps, JSON, field
types, collection names). Numerals are tabular; use `font-tabular` on any
number that changes or sits in a column.

## Tokens (Tailwind utilities)

| Need | Use | Don't |
| --- | --- | --- |
| Page / card / sunken surface | `bg-background` / `bg-card` / `bg-surface-sunken` | hex values, `bg-white`, `bg-zinc-*` |
| Text | `text-foreground`, `text-muted-foreground` | `text-gray-*`, opacity-hacked foreground |
| Border | `border-border` (hairline), `border-border-strong` | `border-gray-*` |
| Primary action | `<Button>` (orange) — **one per view** | orange on anything that isn't "the" action |
| Brand as text | `text-brand-text` (AA on paper & ink) | `text-primary` for body text (fails contrast on light) |
| Brand wash | `bg-primary/14`, `bg-brand-soft` | |
| Status | `text-success` `text-warning` `text-destructive` `text-info`, washes `bg-success/12` | red/green/amber utilities |
| Type sizes | `text-2xs` 10 · `xs` 11 · `sm` 13 (default) · `base` 14 · `lg` 16 · `xl` 19 (page title) · `2xl` 23 (stat numbers) · `3xl`+ (login) | `text-[12px]` |
| Controls | `h-control-xs/sm/md/lg` (24/28/32/36) | `h-7`, `h-9` |
| Spacing | `p-page`, `gap-section`, `max-w-measure`, `h-row` | |
| Radius | `rounded-md/lg/xl` (derived from `--radius`) | `rounded-[7px]` |
| Elevation | `shadow-xs` (card), `shadow-e2` (hover), `shadow-e3` (popover), `shadow-e4/5` (dialog) | hand-written `box-shadow` |
| Motion | `duration-fast/base/slow`, `ease-out-strong`, `animate-rise` | `duration-[200ms]`, `ease-in`, `transition-all` |
| Stacking | `z-sticky` `z-nav` `z-overlay` `z-modal` `z-popover` `z-toast` | `z-50`, `z-[999]` |

Both themes are first-class: always check light **and** dark. Never pair a
hard-coded colour with a token.

## Page primitives

All in `src/components/layout/`. Import path shown.

```tsx
import { PageShell, PageHeader, PageSection } from "@/components/layout/page";
import { StatCard, StatusBadge, StatusDot } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
```

### Standard screen

```tsx
<PageShell width="wide">                       {/* scroll container; width: narrow | wide | full */}
  <PageHeader
    title="Webhooks"                           {/* sentence-case noun */}
    description="POST a JSON payload when a record event fires."
    icon={<Webhook />}
    actions={<Button size="sm"><Plus />New webhook</Button>}   {/* primary action last */}
  />
  <DataTableToolbar
    search={{ value: q, onChange: setQ, placeholder: "Search webhooks…" }}
    filters={<StatusSelect />}
    selection={selected.length ? { count: selected.length, onClear, actions: <Button variant="destructive" size="sm">Delete</Button> } : undefined}
  />
  <PageSection title="Endpoints">
    {isPending ? <ListSkeleton rows={6} />
      : isError ? <ErrorState error={error} onRetry={refetch} />
      : rows.length === 0 ? <EmptyState icon={<Webhook />} title="No webhooks yet" description="…" action={<Button>New webhook</Button>} />
      : <Table>…</Table>}
  </PageSection>
</PageShell>
```

* `PageShell scroll={false}` for screens that own their scrolling (records grid
  with sticky header/footer); give the grid `min-h-0 flex-1`.
* `PageSection bordered` wraps a form/chart in a card; tables bring their own
  border (`rounded-lg border border-border bg-card overflow-hidden`).
* `StatCard` rows: `grid grid-cols-2 gap-2 lg:grid-cols-4`; pass `index` for
  the entrance stagger, `to`/`search` to make it a link, `tone="danger"` only
  for bad news.
* `StatusBadge tone="success|warning|danger|info|brand|neutral"` for any state
  chip (job status, delivery status). Label always states the state — colour is
  never the only signal.
* Loading → skeleton shaped like the content (`Skeleton`, `ListSkeleton`);
  empty → `EmptyState` with the one action that fixes it; failure → `ErrorState`
  in place with retry. Whole-route failures are handled by `RouteError`.

## Shell & navigation

* Sidebar groups: **Data** (collections + SQL/files/backups), **Auth**,
  **Automation**, **AI**, **System**. The registry is `src/lib/nav.ts`; the
  sidebar *and* the command palette read it. Add a screen once, in `NAV_GROUPS`.
* Shortcuts: `⌘K` palette, `⌘B` sidebar, `g` then a letter (`g o` Overview,
  `g q` SQL, `g k` API keys, `g j` Queue, `g l` Logs, `g s` Application). Set
  `chord` on a `NavItem` to add one. Keyboard-initiated actions never animate.
* Breadcrumbs are derived from the URL (`breadcrumbs.tsx`); the page title
  (`PageHeader`) is the h1.
* Settings screens are tabs of a group page (`/settings/<group>?tab=<tab>`).

## Motion

Purposeful and brief. Under 300ms, `ease-out-strong`. Entrances use
`animate-rise` (4px rise + fade, `--i` stagger of 30ms, cap ~8 items). Buttons
press to `scale(0.97)`. Loading uses `Skeleton` (shimmer). Never animate
keyboard-triggered UI (palette, shortcuts) or things seen hundreds of times a
day. `prefers-reduced-motion` collapses all durations globally (index.css);
don't add `motion-safe:` guards for CSS-only animation, do for JS-driven ones.

## Accessibility

* One focus language: 2px brand ring. Controls draw `ring-3 ring-ring/40`;
  everything else gets the global `:focus-visible` outline. Never
  `outline-none` without a replacement ring.
* Every input has a `<Label>` or `aria-label`; icon-only buttons need
  `aria-label`. Errors use `role="alert"` or `aria-describedby`.
* Hit targets ≥ 24px (`size-control-xs`), 28px+ preferred. Skip-link and
  `#main` landmark live in the shell.
* Don't rely on colour alone; keep text on `muted-foreground` ≥ 12px.

## Toasts

`toast.success("Webhook created")` / `toast.error(message)` from `sonner`.
Past-tense, no trailing period for success; errors say what failed and why.
For undoable actions use `toast("Deleted", { action: { label: "Undo", … } })`.
Toasts are tinted per type from the status tokens and auto-dismiss at 4s.

## Do / don't

* Do: sentence case, concrete empty-state copy, mono for identifiers, one
  primary button per view, tabular numerals in columns.
* Don't: nested cards, gradients on content, shadows on tables, decorative
  motion, raw colours/sizes, `transition-all`, a second accent colour.
