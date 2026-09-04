"use client"

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"
import type { CSSProperties, RefObject } from "react"
import { AnimatePresence, motion, useReducedMotion } from "motion/react"
import { ArrowUp } from "lucide-react"

import { cn } from "@/lib/utils"

// Mirrors --ease-out-strong; motion needs the raw bezier, not the CSS var.
const EASE_OUT_STRONG = [0.23, 1, 0.32, 1] as const

const ARRIVE = {
  type: "spring",
  stiffness: 540,
  damping: 34,
  mass: 0.5,
} as const
const INSTANT = { duration: 0 } as const

const useIsoLayoutEffect =
  typeof window === "undefined" ? useEffect : useLayoutEffect

type NewItemsAnchor = "top" | "bottom"

type UseNewItemsOptions = {
  itemCount: number
  anchor?: NewItemsAnchor
  threshold?: number
}

type UseNewItemsResult<T extends HTMLElement> = {
  scrollProps: {
    ref: RefObject<T | null>
    tabIndex: number
    style: CSSProperties
  }
  unread: number
  pinned: boolean

  jump: () => number
}

function useNewItems<T extends HTMLElement = HTMLDivElement>({
  itemCount,
  anchor = "top",
  threshold = 24,
}: UseNewItemsOptions): UseNewItemsResult<T> {
  const ref = useRef<T | null>(null)
  const pinnedRef = useRef(true)
  const prevCount = useRef(itemCount)
  const bottomGap = useRef(0)

  const [unread, setUnread] = useState(0)
  const [pinned, setPinned] = useState(true)
  const reduced = useReducedMotion()

  useEffect(() => {
    const el = ref.current
    if (!el) return

    const read = () =>
      anchor === "bottom"
        ? el.scrollHeight - el.scrollTop - el.clientHeight <= threshold
        : el.scrollTop <= threshold

    const onScroll = () => {
      bottomGap.current = el.scrollHeight - el.scrollTop
      const next = read()
      if (next === pinnedRef.current) return
      pinnedRef.current = next
      setPinned(next)
      if (next) setUnread(0)
    }
    onScroll()
    el.addEventListener("scroll", onScroll, { passive: true })
    return () => el.removeEventListener("scroll", onScroll)
  }, [anchor, threshold])

  useIsoLayoutEffect(() => {
    const el = ref.current
    const added = itemCount - prevCount.current
    prevCount.current = itemCount
    if (!el || added <= 0) return

    if (pinnedRef.current) {
      el.scrollTop = anchor === "bottom" ? el.scrollHeight : 0
      bottomGap.current = el.scrollHeight - el.scrollTop
      return
    }

    // Holding the viewport still while rows land above it keeps the reader's
    // place; only then does the unread counter mean anything.
    if (anchor === "top") {
      const target = el.scrollHeight - bottomGap.current
      if (target > el.scrollTop) el.scrollTop = target
    }
    setUnread((n) => n + added)
  }, [itemCount, anchor])

  const unreadRef = useRef(0)
  unreadRef.current = unread

  const jump = useCallback(() => {
    const el = ref.current
    const caught = unreadRef.current
    if (!el) return caught
    pinnedRef.current = true
    setPinned(true)
    setUnread(0)

    el.focus({ preventScroll: true })
    el.scrollTo({
      top: anchor === "bottom" ? el.scrollHeight : 0,
      behavior: reduced ? "auto" : "smooth",
    })
    return caught
  }, [anchor, reduced])

  return {
    scrollProps: { ref, tabIndex: 0, style: { overflowAnchor: "none" } },
    unread,
    pinned,
    jump,
  }
}

type NewItemsPillProps = {
  count: number
  onJump: () => void
  anchor?: NewItemsAnchor
  label?: (count: number) => string
  max?: number
  className?: string
}

const defaultLabel = (n: number) => `${n} new ${n === 1 ? "item" : "items"}`

function NewItemsPill({
  count,
  onJump,
  anchor = "top",
  label = defaultLabel,
  max = 99,
  className,
}: NewItemsPillProps) {
  const reduced = useReducedMotion()
  const [announced, setAnnounced] = useState(0)

  // Announcing on a delay keeps a burst of arrivals from spamming the
  // live region once per item.
  useEffect(() => {
    if (count === 0) {
      setAnnounced(0)
      return
    }
    const t = setTimeout(() => setAnnounced(count), 700)
    return () => clearTimeout(t)
  }, [count])

  const phrase = (n: number) => (n > max ? `${max}+ new items` : label(n))
  const text = phrase(count)
  const off = anchor === "bottom" ? 10 : -10

  return (
    <div
      data-slot="new-items-pill"
      className={cn(
        "pointer-events-none absolute inset-x-0 z-sticky flex justify-center",
        anchor === "bottom" ? "bottom-2" : "top-2",
        className
      )}
    >
      <AnimatePresence initial={false}>
        {count > 0 && (
          <motion.button
            type="button"
            onClick={onJump}
            aria-label={text}
            initial={
              reduced ? { opacity: 0 } : { opacity: 0, scale: 0.94, y: off }
            }
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={
              reduced
                ? { opacity: 0, transition: INSTANT }
                : {
                    opacity: 0,
                    scale: 0.96,
                    y: off * 0.5,
                    transition: { duration: 0.16, ease: EASE_OUT_STRONG },
                  }
            }
            transition={
              reduced
                ? INSTANT
                : {
                    ...ARRIVE,
                    opacity: { duration: 0.16, ease: EASE_OUT_STRONG },
                  }
            }
            className="pointer-events-auto inline-flex h-control-sm items-center gap-1.5 rounded-full bg-primary pr-3 pl-2.5 text-xs font-medium text-primary-foreground shadow-e3 transition-colors select-none outline-none hover:bg-primary/90 focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            <ArrowUp
              aria-hidden="true"
              className={cn("size-3.5", anchor === "bottom" && "rotate-180")}
            />
            <span className="font-tabular" aria-hidden="true">
              {text}
            </span>
          </motion.button>
        )}
      </AnimatePresence>
      <span role="status" aria-live="polite" className="sr-only">
        {announced > 0 ? phrase(announced) : ""}
      </span>
    </div>
  )
}

export { NewItemsPill, useNewItems }
export type {
  NewItemsAnchor,
  UseNewItemsOptions,
  UseNewItemsResult,
  NewItemsPillProps,
}
