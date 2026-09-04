import { cn } from "@/lib/utils";

/**
 * The Cratebase mark: a crate drawn in one continuous outline with its three
 * interior edges implied. It inherits `currentColor` so it can sit on the
 * sidebar, in a dropdown, or at display size on the login screen without a
 * second asset.
 *
 * `animated` runs the one non-user-triggered motion in the product: the
 * interior edges draw themselves in once, on the login screen, on mount.
 * Everywhere else the mark is static.
 */
export function CratebaseMark({
  className,
  animated = false,
}: {
  className?: string;
  animated?: boolean;
}) {
  return (
    <svg
      viewBox="0 0 32 32"
      fill="none"
      aria-hidden="true"
      className={cn("size-6 text-primary", className)}
    >
      <path
        d="M16 5 27 10.8v10.4L16 27 5 21.2V10.8L16 5Z"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinejoin="round"
      />
      <g
        stroke="currentColor"
        strokeWidth={1.4}
        strokeLinejoin="round"
        opacity={0.55}
        className={
          animated
            ? "[stroke-dasharray:26] motion-safe:animate-[crate-draw_700ms_var(--ease-out-strong)_120ms_backwards]"
            : undefined
        }
      >
        <path d="M16 5v22" />
        <path d="M5 10.8 16 16l11-5.2" />
        <path d="M5 21.2 16 16l11 5.2" />
      </g>
    </svg>
  );
}

/** Mark plus wordmark, at the size the sidebar header and login use. */
export function CratebaseWordmark({
  className,
  animated = false,
}: {
  className?: string;
  animated?: boolean;
}) {
  return (
    <span className={cn("flex items-center gap-2", className)}>
      <CratebaseMark className="size-5" animated={animated} />
      <span className="text-sm font-medium tracking-tight">Cratebase</span>
    </span>
  );
}
