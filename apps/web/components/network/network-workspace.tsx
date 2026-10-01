"use client"

import { createContext, useContext, useEffect, useState, type ReactNode } from "react"

import { AccessDraftProvider } from "@/components/access/draft-store"
import { ConnectionsDraftProvider } from "@/components/connections/draft-store"
import { RoutingDraftProvider } from "@/components/routing/draft-store"
import { NetworkProvider, useNetwork } from "@/components/shell/network-context"
import { AccessDraftRegistry } from "@/lib/access-draft-registry"
import { ConnectionsDraftRegistry } from "@/lib/connections-draft-registry"
import { RoutingDraftRegistry } from "@/lib/routing-draft-registry"

type NetworkDrafts = {
  access: AccessDraftRegistry
  connections: ConnectionsDraftRegistry
  routing: RoutingDraftRegistry
}

type Vault = {
  drafts: (networkId: string) => NetworkDrafts
  /** Logout: drops every network's unsaved work. */
  reset: () => void
  /** Whether a network keeps unsaved work (read when a menu opens; not reactive). */
  hasDrafts: (networkId: string) => boolean
}

const VaultContext = createContext<Vault | null>(null)

/**
 * In-memory, per-network draft registries for the whole signed-in console.
 * It sits above the `/<network>` segment so switching networks neither drops
 * nor mixes drafts: each network keeps its own text, comments, unapplied
 * forms and CAS base. Nothing is persisted to storage.
 */
export function DraftVault({ children }: { children: ReactNode }) {
  const [registries] = useState(() => new Map<string, NetworkDrafts>())
  const [vault] = useState<Vault>(() => ({
    drafts(networkId) {
      let drafts = registries.get(networkId)
      if (!drafts) {
        drafts = { access: new AccessDraftRegistry(), connections: new ConnectionsDraftRegistry(), routing: new RoutingDraftRegistry() }
        registries.set(networkId, drafts)
      }
      return drafts
    },
    reset() {
      for (const drafts of registries.values()) {
        drafts.access.reset()
        drafts.connections.reset()
        drafts.routing.reset()
      }
    },
    hasDrafts(networkId) {
      const drafts = registries.get(networkId)
      return drafts !== undefined && (drafts.access.hasDraft() || drafts.connections.hasDraft() || drafts.routing.hasDraft())
    },
  }))
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if ([...registries.values()].some((drafts) => drafts.access.hasDraft() || drafts.connections.hasDraft() || drafts.routing.hasDraft())) {
        event.preventDefault()
      }
    }
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [registries])
  return <VaultContext.Provider value={vault}>{children}</VaultContext.Provider>
}

export function useDraftVault(): Vault {
  const vault = useContext(VaultContext)
  if (!vault) throw new Error("Draft vault is unavailable")
  return vault
}

/** One network's context: its ID and routes, plus that network's draft stores. */
export function NetworkWorkspace({ networkId, name, children }: { networkId: string; name: string | null; children: ReactNode }) {
  const drafts = useDraftVault().drafts(networkId)
  return (
    <NetworkProvider networkId={networkId} name={name}>
      <RoutingDraftProvider key={networkId} registry={drafts.routing}>
        <ConnectionsDraftProvider registry={drafts.connections}>
          <AccessDraftProvider registry={drafts.access}>{children}</AccessDraftProvider>
        </ConnectionsDraftProvider>
      </RoutingDraftProvider>
    </NetworkProvider>
  )
}

/** The network the current page was rendered for; actions are bound to it. */
export function useNetworkId(): string {
  return useNetwork().networkId
}
