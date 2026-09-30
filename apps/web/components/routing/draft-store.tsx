"use client"

import type { NodeSummary, PolicySnapshot } from "@meshway/control-client"
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"

import { policyKey, toPolicy, type PolicyDraft } from "@/lib/policy-model"

/**
 * An unsaved routing draft together with the exact snapshot it was based on.
 * Restoring it restores the CAS base too, so a draft never silently moves
 * onto a newer revision.
 */
export type StoredDraft = {
  base: PolicySnapshot
  nodes: NodeSummary[]
  draft: PolicyDraft
  /** Next suffix for `new-<n>` rule keys, so restored keys never collide. */
  nextKey: number
}

type DraftStore = {
  read: () => StoredDraft | null
  write: (value: StoredDraft) => void
  clear: () => void
  /** Clears only if the stored draft is exactly `key` (a policy just saved). */
  clearIfPolicy: (key: string) => void
  hasDraft: boolean
}

const DraftStoreContext = createContext<DraftStore | null>(null)

/**
 * In-memory only, scoped to the console layout: in-app navigation (links,
 * the command menu, browser back/forward) unmounts the editor but keeps the
 * draft. A full reload, logout or leaving the console discards it. Nothing is
 * persisted to storage and it never contains credentials.
 */
export function RoutingDraftProvider({ children }: { children: ReactNode }) {
  const stored = useRef<StoredDraft | null>(null)
  const [hasDraft, setHasDraft] = useState(false)

  const read = useCallback(() => stored.current, [])
  const write = useCallback((value: StoredDraft) => {
    stored.current = value
    setHasDraft(true)
  }, [])
  const clear = useCallback(() => {
    stored.current = null
    setHasDraft(false)
  }, [])
  const clearIfPolicy = useCallback(
    (key: string) => {
      const current = stored.current
      if (current && policyKey(toPolicy(current.draft)) === key) clear()
    },
    [clear]
  )

  // A kept draft lives only in memory, so a reload or tab close on any
  // console page would drop it: warn while one exists.
  useEffect(() => {
    if (!hasDraft) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [hasDraft])

  const value = useMemo(
    () => ({ read, write, clear, clearIfPolicy, hasDraft }),
    [read, write, clear, clearIfPolicy, hasDraft]
  )
  return <DraftStoreContext.Provider value={value}>{children}</DraftStoreContext.Provider>
}

export function useRoutingDraftStore(): DraftStore {
  const store = useContext(DraftStoreContext)
  if (!store) throw new Error("useRoutingDraftStore must be used inside RoutingDraftProvider")
  return store
}
