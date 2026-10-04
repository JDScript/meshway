import { useSyncExternalStore } from "react"

/** Whether `query` matches now, following its changes. */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const list = window.matchMedia(query)
      list.addEventListener("change", onChange)
      return () => list.removeEventListener("change", onChange)
    },
    () => window.matchMedia(query).matches
  )
}

const MOBILE_BREAKPOINT = 768

/** Small screens: sheets instead of floating surfaces. */
export const useIsMobile = () =>
  useMediaQuery(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`)

/** The person asked for less motion. */
export const usePrefersReducedMotion = () =>
  useMediaQuery("(prefers-reduced-motion: reduce)")
