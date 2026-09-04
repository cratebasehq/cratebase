import { Component, type ErrorInfo, type ReactNode } from "react";
import { Link, useRouter } from "@tanstack/react-router";
import { CircleAlert, FileQuestion, RotateCw } from "lucide-react";
import { CratebaseMark } from "@/components/brand/cratebase-mark";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { describeFailure } from "@/lib/api";

/* ------------------------------------------------------------------------ *
 * Route-level states
 * ------------------------------------------------------------------------ */

/** Rendered by the router for any error thrown in a loader or a screen.
 * Before this, a failing route rendered nothing at all. */
export function RouteError({ error, reset }: { error: Error; reset?: () => void }) {
  const router = useRouter();
  const failure = describeFailure(error);

  return (
    <div className="flex flex-1 items-center justify-center p-page">
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon" className="bg-destructive/10 text-destructive">
            <CircleAlert />
          </EmptyMedia>
          <EmptyTitle>{failure.title}</EmptyTitle>
          <EmptyDescription>
            {failure.detail || "Something in this screen failed while loading."}
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button
            size="sm"
            onClick={() => {
              reset?.();
              void router.invalidate();
            }}
          >
            <RotateCw />
            Try again
          </Button>
          <ErrorDetails error={error} />
        </EmptyContent>
      </Empty>
    </div>
  );
}

/** The skeleton the router shows while a route resolves. Shaped like the
 * screens it stands in for — a header strip over rows — so the layout does
 * not jump when the real content lands. */
export function RoutePending() {
  return (
    <div aria-busy="true" aria-label="Loading" className="flex flex-1 flex-col">
      <div className="flex h-topbar shrink-0 items-center gap-3 border-b border-border px-page">
        <Skeleton className="h-4 w-40" />
        <Skeleton className="ml-auto h-control-sm w-24" />
      </div>
      <div className="flex flex-col">
        {Array.from({ length: 10 }, (_, index) => (
          <div
            key={index}
            className="flex h-row items-center gap-4 border-b border-border px-page"
            style={{ opacity: 1 - index * 0.08 }}
          >
            <Skeleton className="h-3 w-24" />
            <Skeleton className="h-3 w-40" />
            <Skeleton className="h-3 w-16" />
            <Skeleton className="ml-auto h-3 w-28" />
          </div>
        ))}
      </div>
    </div>
  );
}

export function RouteNotFound() {
  return (
    <div className="flex flex-1 items-center justify-center p-page">
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <FileQuestion />
          </EmptyMedia>
          <EmptyTitle>No such page</EmptyTitle>
          <EmptyDescription>
            The address you followed doesn't match any screen in this dashboard.
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button asChild size="sm" variant="outline">
            <Link to="/">Back to overview</Link>
          </Button>
        </EmptyContent>
      </Empty>
    </div>
  );
}

function ErrorDetails({ error }: { error: unknown }) {
  const text = error instanceof Error ? (error.stack ?? error.message) : String(error);

  return (
    <Collapsible className="w-full">
      <CollapsibleTrigger asChild>
        <Button variant="ghost" size="xs" className="text-muted-foreground">
          Technical details
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <pre className="mt-2 max-h-56 overflow-auto rounded-md border border-border bg-surface-sunken p-2 text-left font-mono text-2xs leading-4 text-muted-foreground">
          {text}
        </pre>
      </CollapsibleContent>
    </Collapsible>
  );
}

/* ------------------------------------------------------------------------ *
 * Global boundary
 * ------------------------------------------------------------------------ */

interface BoundaryState {
  error: Error | null;
}

/**
 * The last line of defence, outside the router: a render error anywhere in
 * the tree used to blank the page with no way back.
 *
 * A class component because React still offers no hook equivalent of
 * `componentDidCatch`.
 */
export class AppErrorBoundary extends Component<{ children: ReactNode }, BoundaryState> {
  state: BoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): BoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Unhandled error in the admin dashboard", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex min-h-svh flex-col items-center justify-center gap-6 bg-background p-page">
        <CratebaseMark className="size-6" />
        <Empty className="border-none">
          <EmptyHeader>
            <EmptyTitle className="text-lg">The dashboard crashed</EmptyTitle>
            <EmptyDescription>
              This is a bug in the admin UI, not in your data. Reloading usually recovers it.
            </EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <Button size="sm" onClick={() => window.location.reload()}>
              <RotateCw />
              Reload the dashboard
            </Button>
            <ErrorDetails error={error} />
          </EmptyContent>
        </Empty>
      </div>
    );
  }
}
