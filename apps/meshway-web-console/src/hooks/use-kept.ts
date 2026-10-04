import { useEffect, useState } from "react"

/**
 * `value` while `shown`, and the last value shown once it is not: a surface
 * that is leaving keeps its content until it is gone.
 */
export function useKeptWhile<T>(shown: boolean, value: T): T {
  const [kept, setKept] = useState(() => value)
  if (shown && !Object.is(kept, value)) setKept(() => value)
  return shown ? value : kept
}

/** Whether something that `open`s should still be mounted: until `ms` after it closes, so it can animate out. */
export function useUnmountAfter(open: boolean, ms: number): boolean {
  const [mounted, setMounted] = useState(open)
  if (open && !mounted) setMounted(true)
  useEffect(() => {
    if (open || !mounted) return
    const timer = window.setTimeout(() => setMounted(false), ms)
    return () => window.clearTimeout(timer)
  }, [open, mounted, ms])
  return mounted
}
