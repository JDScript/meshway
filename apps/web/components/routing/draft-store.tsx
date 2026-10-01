"use client"

import {
  createContext,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react"

import { RoutingDraftRegistry } from "@/lib/routing-draft-registry"

export type { Owner, StoredRoutingDraft } from "@/lib/routing-draft-registry"

const RoutingDraftContext = createContext<RoutingDraftRegistry | null>(null)

/**
 * In-memory only, scoped to the console layout: in-app navigation (links,
 * the command menu, browser back / forward) unmounts the editor but keeps the
 * draft and any unapplied rule-builder edits. A full reload, logout or
 * leaving the console discards them. Nothing is persisted to storage and it
 * never contains credentials.
 */
export function RoutingDraftProvider({ children }: { children: ReactNode }) {
  const [registry] = useState(() => new RoutingDraftRegistry())
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

  return <RoutingDraftContext.Provider value={registry}>{children}</RoutingDraftContext.Provider>
}

export function useRoutingDraftStore(): RoutingDraftRegistry {
  const registry = useContext(RoutingDraftContext)
  if (!registry) throw new Error("useRoutingDraftStore must be used inside RoutingDraftProvider")
  return registry
}

/** Whether unsaved routing work is kept (for navigation badges). */
export function useRoutingHasDraft(): boolean {
  const registry = useRoutingDraftStore()
  return useSyncExternalStore(
    (listener) => registry.subscribe(listener),
    () => registry.hasDraft(),
    () => false
  )
}
