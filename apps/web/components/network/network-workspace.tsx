"use client"

import { createContext, useContext, useEffect, useState, type ReactNode } from "react"
import { AccessDraftProvider } from "@/components/access/draft-store"
import { ConnectionsDraftProvider } from "@/components/connections/draft-store"
import { RoutingDraftProvider } from "@/components/routing/draft-store"
import { AccessDraftRegistry } from "@/lib/access-draft-registry"
import { ConnectionsDraftRegistry } from "@/lib/connections-draft-registry"
import { RoutingDraftRegistry } from "@/lib/routing-draft-registry"

const NetworkContext = createContext<string | null>(null)

export function useNetworkId(): string {
  const id = useContext(NetworkContext)
  if (!id) throw new Error("Network workspace is unavailable")
  return id
}

/** Each network keeps its own draft text, comments, ownership and CAS base. */
export function NetworkWorkspace({ networkId, children }: { networkId: string; children: ReactNode }) {
  const [registries] = useState(() => new Map<string, {
    access: AccessDraftRegistry; connections: ConnectionsDraftRegistry; routing: RoutingDraftRegistry
  }>())
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if ([...registries.values()].some(drafts => drafts.access.hasDraft() || drafts.connections.hasDraft() || drafts.routing.hasDraft())) event.preventDefault()
    }
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [registries])
  let drafts = registries.get(networkId)
  if (!drafts) {
    drafts = { access: new AccessDraftRegistry(), connections: new ConnectionsDraftRegistry(), routing: new RoutingDraftRegistry() }
    registries.set(networkId, drafts)
  }
  return <NetworkContext.Provider value={networkId}>
    <RoutingDraftProvider key={networkId} registry={drafts.routing}>
      <ConnectionsDraftProvider registry={drafts.connections}>
        <AccessDraftProvider registry={drafts.access}>{children}</AccessDraftProvider>
      </ConnectionsDraftProvider>
    </RoutingDraftProvider>
  </NetworkContext.Provider>
}
