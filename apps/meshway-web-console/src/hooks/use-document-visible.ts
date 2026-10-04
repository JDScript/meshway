import { useSyncExternalStore } from "react"

const subscribe = (onChange: () => void) => {
  document.addEventListener("visibilitychange", onChange)
  return () => document.removeEventListener("visibilitychange", onChange)
}

/** Whether this tab is in front. */
export const useDocumentVisible = (): boolean =>
  useSyncExternalStore(subscribe, () => document.visibilityState === "visible")
