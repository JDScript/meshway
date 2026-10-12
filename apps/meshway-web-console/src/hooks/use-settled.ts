import { useEffect, useState } from "react"

/** `value` once it has stayed the same for `ms` (what a search reads as one types). */
export function useSettled<T>(value: T, ms = 250): T {
  const [settled, setSettled] = useState(value)
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(value), ms)
    return () => window.clearTimeout(timer)
  }, [value, ms])
  return settled
}
