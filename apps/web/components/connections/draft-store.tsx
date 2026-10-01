"use client"

import {
  createContext,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react"

import { ConnectionsDraftRegistry } from "@/lib/connections-draft-registry"

export type { Owner, StoredConnectionsDraft } from "@/lib/connections-draft-registry"

const ConnectionsDraftContext = createContext<ConnectionsDraftRegistry | null>(null)

/**
 * Sibling of the routing draft store, with the same lifetime: in-memory only
 * and scoped to the console layout. In-app navigation (links, the command
 * menu, browser back / forward) keeps the draft and any unapplied sheet
 * edits; a full reload, logout or leaving the console discards them. Nothing
 * is persisted to storage and it never contains credentials.
 */
export function ConnectionsDraftProvider({ children }: { children: ReactNode }) {
  const [registry] = useState(() => new ConnectionsDraftRegistry())
  const hasDraft = useSyncExternalStore(
    (listener) => registry.subscribe(listener),
    () => registry.hasDraft(),
    () => false
  )

  // Kept work lives only in memory, so a reload or tab close on any console
  // page would drop it: warn while any exists.
  useEffect(() => {
    if (!hasDraft) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [hasDraft])

  return (
    <ConnectionsDraftContext.Provider value={registry}>{children}</ConnectionsDraftContext.Provider>
  )
}

export function useConnectionsDraftStore(): ConnectionsDraftRegistry {
  const registry = useContext(ConnectionsDraftContext)
  if (!registry) {
    throw new Error("useConnectionsDraftStore must be used inside ConnectionsDraftProvider")
  }
  return registry
}
