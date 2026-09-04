"use client"

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react"
import type {
  ChangeEvent,
  ClipboardEvent as ReactClipboardEvent,
  KeyboardEvent as ReactKeyboardEvent,
} from "react"
import { AnimatePresence, motion, useReducedMotion } from "motion/react"
import { X } from "lucide-react"

import { cn } from "@/lib/utils"

// Bezier literals mirror the CSS easing tokens; motion needs the raw curve.
const EASE_IN_STRONG = [0.4, 0, 1, 1] as const

const CROSSFADE = {
  type: "spring",
  stiffness: 260,
  damping: 34,
  mass: 0.8,
} as const
const CHIP = { type: "spring", stiffness: 700, damping: 46, mass: 0.5 } as const
const EXIT = { duration: 0.18, ease: EASE_IN_STRONG } as const
const INSTANT = { duration: 0 } as const

const clean = (raw: string) => raw.trim().replace(/\s+/g, " ")

const splitter = (separators: string[]) =>
  new RegExp(
    `[${separators.map((s) => s.replace(/[\\\]^-]/g, "\\$&")).join("")}\\n\\r\\t]+`
  )

type TagRejection = "duplicate" | "limit" | "invalid"

type UseTagInputOptions = {
  value?: string[]
  defaultValue?: string[]
  onChange?: (tags: string[]) => void
  max?: number
  separators?: string[]
  allowDuplicates?: boolean
  validate?: (candidate: string, tags: string[]) => boolean
}

type Rejection = { reason: TagRejection; tag: string; visible: boolean }

function useTagInput({
  value,
  defaultValue,
  onChange,
  max,
  separators = [","],
  allowDuplicates = false,
  validate,
}: UseTagInputOptions = {}) {
  const [internal, setInternal] = useState<string[]>(() => defaultValue ?? [])
  const [draft, setDraft] = useState("")
  const [armed, setArmed] = useState(-1)
  const [rejection, setRejection] = useState<Rejection | null>(null)
  const [flashed, setFlashed] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState("")

  const controlled = value !== undefined
  const tags = value ?? internal
  const armedIndex = armed >= tags.length ? -1 : armed

  const emit = useRef(onChange)
  emit.current = onChange
  const check = useRef(validate)
  check.current = validate

  const rejectTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const flashTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(
    () => () => {
      if (rejectTimer.current) clearTimeout(rejectTimer.current)
      if (flashTimer.current) clearTimeout(flashTimer.current)
    },
    []
  )

  const dismiss = useCallback(() => {
    if (rejectTimer.current) clearTimeout(rejectTimer.current)
    rejectTimer.current = null
    setRejection((prev) =>
      prev && prev.visible ? { ...prev, visible: false } : prev
    )
  }, [])

  const refuse = useCallback(
    (reason: TagRejection, tag: string) => {
      if (rejectTimer.current) clearTimeout(rejectTimer.current)
      setRejection({ reason, tag, visible: true })
      rejectTimer.current = setTimeout(() => {
        setRejection((prev) => (prev ? { ...prev, visible: false } : prev))
      }, 2400)

      setAnnouncement(
        reason === "duplicate"
          ? `${tag} is already in the list.`
          : reason === "limit"
            ? `That is the limit of ${max} tags.`
            : `${tag} is not allowed here.`
      )

      if (reason !== "duplicate") return
      if (flashTimer.current) clearTimeout(flashTimer.current)
      setFlashed(tag)
      flashTimer.current = setTimeout(() => setFlashed(null), 460)
    },
    [max]
  )

  const apply = useCallback(
    (next: string[]) => {
      if (!controlled) setInternal(next)
      emit.current?.(next)
    },
    [controlled]
  )

  const add = useCallback(
    (raws: string[]) => {
      const next = [...tags]
      let added = 0
      let failure: { reason: TagRejection; tag: string } | null = null

      for (const raw of raws) {
        const candidate = clean(raw)
        if (!candidate) continue

        if (max !== undefined && next.length >= max) {
          failure = { reason: "limit", tag: candidate }
          break
        }

        if (!allowDuplicates) {
          const twin = next.find(
            (t) => t.toLowerCase() === candidate.toLowerCase()
          )
          if (twin) {
            failure = { reason: "duplicate", tag: twin }
            continue
          }
        }

        if (check.current && !check.current(candidate, next)) {
          failure = { reason: "invalid", tag: candidate }
          continue
        }

        next.push(candidate)
        added += 1
      }

      if (added > 0) {
        apply(next)
        setDraft("")
        setArmed(-1)
        dismiss()
        setAnnouncement(
          `${added === 1 ? next[next.length - 1] : `${added} tags`} added, ${next.length} total.`
        )
      }

      if (failure) refuse(failure.reason, failure.tag)
      return added > 0
    },
    [tags, max, allowDuplicates, apply, dismiss, refuse]
  )

  const removeAt = useCallback(
    (index: number) => {
      if (index < 0 || index >= tags.length) return
      const gone = tags[index]
      const next = tags.filter((_, i) => i !== index)
      apply(next)
      setArmed(-1)
      dismiss()
      setAnnouncement(`${gone} removed, ${next.length} left.`)
    },
    [tags, apply, dismiss]
  )

  const arm = useCallback(
    (index: number) => {
      setArmed(index)
      setAnnouncement(
        `${tags[index]} selected, press Backspace again to remove it.`
      )
    },
    [tags]
  )

  const inputProps = {
    value: draft,
    onChange: (e: ChangeEvent<HTMLInputElement>) => {
      setDraft(e.target.value)
      setArmed(-1)
      dismiss()
    },
    onKeyDown: (e: ReactKeyboardEvent<HTMLInputElement>) => {
      if (e.nativeEvent.isComposing) return

      if (e.key === "Enter" || separators.includes(e.key)) {
        e.preventDefault()
        add([draft])
        return
      }

      if (e.key === "Backspace" && draft === "") {
        e.preventDefault()
        if (e.repeat) return
        if (armedIndex >= 0) removeAt(armedIndex)
        else if (tags.length > 0) arm(tags.length - 1)
        return
      }

      if (e.key === "Delete" && armedIndex >= 0) {
        e.preventDefault()
        if (e.repeat) return
        removeAt(armedIndex)
        return
      }

      if (e.key === "ArrowLeft") {
        const start = e.currentTarget.selectionStart
        const end = e.currentTarget.selectionEnd
        if (start !== 0 || end !== 0 || tags.length === 0) return
        e.preventDefault()
        arm(armedIndex < 0 ? tags.length - 1 : Math.max(0, armedIndex - 1))
        return
      }

      if (e.key === "ArrowRight" && armedIndex >= 0) {
        e.preventDefault()
        if (armedIndex >= tags.length - 1) setArmed(-1)
        else arm(armedIndex + 1)
        return
      }

      if (e.key === "Escape" && armedIndex >= 0) {
        e.preventDefault()
        setArmed(-1)
      }
    },
    onPaste: (e: ReactClipboardEvent<HTMLInputElement>) => {
      const text = e.clipboardData.getData("text")
      const pattern = splitter(separators)
      if (!pattern.test(text)) return
      e.preventDefault()
      add(text.split(pattern))
    },
    onBlur: () => setArmed(-1),
  }

  return {
    tags,
    draft,
    setDraft,
    armedIndex,
    flashed,
    rejection,
    announcement,
    inputProps,
    add,
    removeAt,
    max,
  }
}

type TagInputProps = UseTagInputOptions & {
  label?: string
  placeholder?: string
  hint?: string
  className?: string
}

function TagInput({
  label,
  placeholder = "Add a tag",
  hint = "Enter adds · Backspace removes",
  className,
  ...options
}: TagInputProps) {
  const {
    tags,
    draft,
    armedIndex,
    flashed,
    rejection,
    announcement,
    inputProps,
    removeAt,
    max,
  } = useTagInput(options)

  const reduced = useReducedMotion()
  const inputRef = useRef<HTMLInputElement>(null)
  const uid = useId()
  const inputId = `${uid}-tag-input`
  const hintId = `${uid}-tag-hint`

  // Duplicates are legal when allowDuplicates is set, so chips need a key
  // that survives two identical labels sitting side by side.
  const rows = useMemo(() => {
    const seen = new Map<string, number>()
    return tags.map((tag) => {
      const n = seen.get(tag) ?? 0
      seen.set(tag, n + 1)
      return { tag, key: n === 0 ? tag : `${tag}#${n}` }
    })
  }, [tags])

  const message = !rejection
    ? ""
    : rejection.reason === "duplicate"
      ? `${rejection.tag} is already in the list`
      : rejection.reason === "limit"
        ? `That is the limit of ${max} tags`
        : `${rejection.tag} is not allowed here`

  const showMessage = rejection?.visible === true

  return (
    <div data-slot="tag-input" className={cn("w-full", className)}>
      {label ? (
        <label
          htmlFor={inputId}
          className="mb-1.5 block text-xs font-medium text-foreground"
        >
          {label}
        </label>
      ) : null}

      <ul
        data-slot="tag-input-field"
        onPointerDown={(e) => {
          if (e.target !== e.currentTarget) return
          e.preventDefault()
          inputRef.current?.focus()
        }}
        className="relative flex max-h-28 min-h-control-md list-none flex-wrap items-center gap-1.5 overflow-y-auto overscroll-contain rounded-lg border border-input bg-transparent p-1 text-sm transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50 dark:bg-input/30"
      >
        <AnimatePresence initial={false} mode="popLayout">
          {rows.map(({ tag, key }, index) => {
            const lit = armedIndex === index || flashed === tag
            return (
              <motion.li
                key={key}
                role="listitem"
                layout="position"
                initial={reduced ? false : { opacity: 0, scale: 0.9 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={
                  reduced
                    ? { opacity: 0, transition: INSTANT }
                    : { opacity: 0, scale: 0.9, transition: EXIT }
                }
                transition={reduced ? INSTANT : { default: CHIP, layout: CHIP }}
                className={cn(
                  "flex h-control-xs max-w-full shrink-0 items-center gap-1 rounded-md pr-1 pl-2 text-xs transition-colors select-none",
                  lit
                    ? "bg-primary text-primary-foreground"
                    : "bg-secondary text-secondary-foreground"
                )}
              >
                <span className="truncate">{tag}</span>
                <button
                  type="button"
                  tabIndex={-1}
                  aria-label={`Remove ${tag}`}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => {
                    removeAt(index)
                    inputRef.current?.focus()
                  }}
                  className={cn(
                    "grid size-3.5 shrink-0 place-items-center rounded-sm transition-colors",
                    lit
                      ? "text-primary-foreground/70 hover:text-primary-foreground"
                      : "text-muted-foreground hover:text-foreground"
                  )}
                >
                  <X aria-hidden className="size-2.5" />
                </button>
              </motion.li>
            )
          })}
        </AnimatePresence>

        <motion.li
          layout={reduced ? false : "position"}
          transition={reduced ? INSTANT : CHIP}
          className="relative flex h-control-xs flex-1"
        >
          {/* Invisible twin sizes the field to the draft so the caret sits
              beside the last chip instead of wrapping to its own line. */}
          <span
            aria-hidden
            className="pointer-events-none invisible max-w-56 overflow-hidden px-1 text-sm whitespace-pre"
          >
            {draft || placeholder}
          </span>
          <input
            {...inputProps}
            ref={inputRef}
            id={inputId}
            type="text"
            aria-describedby={hintId}
            aria-label={label ? undefined : "Tags"}
            placeholder={placeholder}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="done"
            className="absolute inset-0 h-full w-full bg-transparent px-1 text-sm text-foreground outline-none placeholder:text-muted-foreground"
          />
        </motion.li>
      </ul>

      <div className="mt-1.5 flex items-baseline justify-between gap-3">
        <div className="grid min-w-0 flex-1">
          <motion.p
            id={hintId}
            initial={false}
            animate={{ opacity: showMessage ? 0 : 1 }}
            transition={reduced ? INSTANT : CROSSFADE}
            className="col-start-1 row-start-1 truncate text-xs text-muted-foreground"
          >
            {hint}
          </motion.p>
          <motion.p
            aria-hidden
            initial={false}
            animate={{ opacity: showMessage ? 1 : 0 }}
            transition={reduced ? INSTANT : CROSSFADE}
            className="col-start-1 row-start-1 truncate text-xs text-destructive"
          >
            {message}
          </motion.p>
        </div>

        {max === undefined ? null : (
          <p className="shrink-0 font-tabular text-xs text-muted-foreground">
            {/* The widest possible count reserves the space so the counter
                never shifts the hint as tags are added. */}
            <span className="inline-grid justify-items-end">
              <span aria-hidden className="col-start-1 row-start-1 invisible">
                {max}
              </span>
              <span className="col-start-1 row-start-1">{tags.length}</span>
            </span>
            <span> / {max}</span>
          </p>
        )}
      </div>

      <span
        role="status"
        aria-live="polite"
        aria-atomic="true"
        className="sr-only"
      >
        {announcement}
      </span>
    </div>
  )
}

export { TagInput, useTagInput }
export type { TagInputProps, UseTagInputOptions, TagRejection }
