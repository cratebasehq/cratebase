"use client"

import { useId } from "react"

import { cn } from "@/lib/utils"

/* index.css owns the shared token layer, so this one-off sweep for the
 * indeterminate state ships with the component that needs it. */
const SWEEP_KEYFRAMES =
  "@keyframes cb-progress-sweep{from{transform:translateX(-100%)}to{transform:translateX(320%)}}"

type ProgressProps = {
  value: number | null
  max?: number
  label?: string
  className?: string
}

function Progress({ value, max = 100, label, className }: ProgressProps) {
  const labelId = useId()
  const indeterminate = value === null
  const fraction =
    indeterminate || max <= 0 ? 0 : Math.min(1, Math.max(0, value / max))
  const percent = Math.round(fraction * 100)

  return (
    <div data-slot="progress" className={cn("w-full", className)}>
      <style>{SWEEP_KEYFRAMES}</style>

      {label ? (
        <div className="mb-1.5 flex items-baseline justify-between gap-3">
          <span id={labelId} className="truncate text-xs text-muted-foreground">
            {label}
          </span>
          <span
            aria-hidden
            className="shrink-0 font-tabular text-xs text-muted-foreground"
          >
            {indeterminate ? "—" : `${percent}%`}
          </span>
        </div>
      ) : null}

      <div
        role="progressbar"
        aria-labelledby={label ? labelId : undefined}
        aria-valuemin={0}
        aria-valuemax={max}
        aria-valuenow={
          indeterminate ? undefined : Math.round(fraction * max * 100) / 100
        }
        aria-valuetext={indeterminate ? undefined : `${percent}%`}
        data-state={indeterminate ? "indeterminate" : "determinate"}
        className="relative h-1 overflow-hidden rounded-full bg-muted"
      >
        {indeterminate ? (
          <span
            aria-hidden
            className="absolute inset-y-0 left-0 w-2/5 animate-[cb-progress-sweep_1.4s_var(--ease-in-out-strong)_infinite] rounded-full bg-primary motion-reduce:w-1/3 motion-reduce:animate-pulse"
          />
        ) : (
          <span
            aria-hidden
            style={{ width: `${percent}%` }}
            className="block h-full rounded-full bg-primary transition-[width] duration-base ease-out-strong"
          />
        )}
      </div>
    </div>
  )
}

export { Progress }
export type { ProgressProps }
