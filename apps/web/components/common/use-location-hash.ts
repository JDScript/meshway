"use client"

import { useSyncExternalStore } from "react"

const subscribe = (callback: () => void) => {
  window.addEventListener("hashchange", callback)
  window.addEventListener("popstate", callback)
  return () => {
    window.removeEventListener("hashchange", callback)
    window.removeEventListener("popstate", callback)
  }
}

/** Hash-targeted help stays discoverable through existing diagnostic deep links. */
export function useLocationHash() {
  return useSyncExternalStore(subscribe, () => window.location.hash.slice(1), () => "")
}
