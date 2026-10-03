import { useSyncExternalStore } from "react"

const subscribe = (onChange: () => void) => {
  window.addEventListener("resize", onChange)
  return () => window.removeEventListener("resize", onChange)
}

export function useViewportHeight(): number {
  return useSyncExternalStore(subscribe, () => window.innerHeight)
}
