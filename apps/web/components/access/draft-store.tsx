"use client"

import {
  createContext,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react"

import { AccessDraftRegistry } from "@/lib/access-draft-registry"

export type { Owner, StoredAccessDraft } from "@/lib/access-draft-registry"

const AccessDraftContext = createContext<AccessDraftRegistry | null>(null)

/**
 * Sibling of the routing and connections draft stores, with the same
 * lifetime: in-memory only and scoped to the console layout. Nothing is
 * persisted to storage and it never contains credentials.
 */
export function AccessDraftProvider({ children, registry: supplied }: { children: ReactNode; registry?: AccessDraftRegistry }) {
  const [registry] = useState(() => supplied ?? new AccessDraftRegistry())
  const hasDraft = useSyncExternalStore(
    (listener) => registry.subscribe(listener),
    () => registry.hasDraft(),
    () => false
  )

  useEffect(() => {
    if (!hasDraft) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [hasDraft])

  return <AccessDraftContext.Provider value={registry}>{children}</AccessDraftContext.Provider>
}

export function useAccessDraftStore(): AccessDraftRegistry {
  const registry = useContext(AccessDraftContext)
  if (!registry) throw new Error("useAccessDraftStore must be used inside AccessDraftProvider")
  return registry
}

/** Whether unsaved access work is kept (for navigation badges). */
export function useAccessHasDraft(): boolean {
  const registry = useAccessDraftStore()
  return useSyncExternalStore(
    (listener) => registry.subscribe(listener),
    () => registry.hasDraft(),
    () => false
  )
}
