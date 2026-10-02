"use client"

import { createContext, useContext, useState, useSyncExternalStore, type ReactNode } from "react"

import { ManagedListenerDraftRegistry } from "@/lib/managed-listener-draft-registry"

const Context = createContext<ManagedListenerDraftRegistry | null>(null)

/** One network's unsaved managed-listener forms (in memory; the vault warns before unload). */
export function ManagedListenerDraftProvider({ registry: supplied, children }: { registry?: ManagedListenerDraftRegistry; children: ReactNode }) {
  const [registry] = useState(() => supplied ?? new ManagedListenerDraftRegistry())
  return <Context.Provider value={registry}>{children}</Context.Provider>
}

export function useManagedListenerDrafts(): ManagedListenerDraftRegistry {
  const registry = useContext(Context)
  if (!registry) throw new Error("useManagedListenerDrafts must be used inside ManagedListenerDraftProvider")
  return registry
}

/** Re-renders on every draft write; returns the registry for reads. */
export function useManagedListenerDraftVersion(): ManagedListenerDraftRegistry {
  const registry = useManagedListenerDrafts()
  useSyncExternalStore(
    (listener) => registry.subscribe(listener),
    () => registry.snapshot(),
    () => 0
  )
  return registry
}
