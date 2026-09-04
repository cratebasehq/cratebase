import { useCallback, useEffect, useRef, useState } from "react"

type CopyStatus = "idle" | "copied" | "error"

type UseCopyToClipboardOptions = {
  timeout?: number
  onCopy?: (value: string) => void
  onError?: (reason: unknown) => void
}

/* navigator.clipboard is unavailable on insecure origins and inside some
 * embedded webviews, so a hidden textarea + execCommand stays as the floor. */
function writeFallback(text: string): boolean {
  const area = document.createElement("textarea")
  area.value = text
  area.setAttribute("readonly", "")
  area.style.position = "fixed"
  area.style.top = "0"
  area.style.left = "0"
  area.style.opacity = "0"
  document.body.appendChild(area)

  const selection = document.getSelection()
  const previous =
    selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null

  area.select()
  let ok = false
  try {
    ok = document.execCommand("copy")
  } catch {
    ok = false
  }

  document.body.removeChild(area)
  if (selection && previous) {
    selection.removeAllRanges()
    selection.addRange(previous)
  }
  return ok
}

function useCopyToClipboard({
  timeout = 2000,
  onCopy,
  onError,
}: UseCopyToClipboardOptions = {}) {
  const [status, setStatus] = useState<CopyStatus>("idle")
  // A ticket, not the status alone, so copying twice in a row restarts the
  // reset timer even though the status never leaves "copied".
  const [ticket, setTicket] = useState(0)

  const mounted = useRef(true)
  const copied = useRef(onCopy)
  copied.current = onCopy
  const failed = useRef(onError)
  failed.current = onError

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  const reset = useCallback(() => {
    setStatus("idle")
    setTicket(0)
  }, [])

  const copy = useCallback(async (text: string) => {
    if (!text) return false

    let ok = false
    let reason: unknown = null

    try {
      if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text)
        ok = true
      } else {
        ok = writeFallback(text)
      }
    } catch (error) {
      reason = error
      try {
        ok = writeFallback(text)
      } catch {
        ok = false
      }
    }

    if (!mounted.current) return ok

    setStatus(ok ? "copied" : "error")
    setTicket((t) => t + 1)

    if (ok) copied.current?.(text)
    else failed.current?.(reason)

    return ok
  }, [])

  useEffect(() => {
    if (ticket === 0 || status === "idle") return
    const id = setTimeout(() => setStatus("idle"), timeout)
    return () => clearTimeout(id)
  }, [ticket, status, timeout])

  return { copy, reset, status, copied: status === "copied" }
}

export { useCopyToClipboard }
export type { CopyStatus, UseCopyToClipboardOptions }
