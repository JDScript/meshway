import { useSyncExternalStore } from "react"

const subscribe = (onChange: () => void) => {
  window.addEventListener("resize", onChange)
  return () => window.removeEventListener("resize", onChange)
}

export const useViewportWidth = (): number =>
  useSyncExternalStore(subscribe, () => window.innerWidth)

export const useViewportHeight = (): number =>
  useSyncExternalStore(subscribe, () => window.innerHeight)
