"use client"

import { createContext, useContext, useEffect, useState, type ReactNode } from "react"

import { AccessDraftProvider } from "@/components/access/draft-store"
import { ConnectionsDraftProvider } from "@/components/connections/draft-store"
import { ManagedListenerDraftProvider } from "@/components/managed-listeners/draft-store"
import { RoutingDraftProvider } from "@/components/routing/draft-store"
import { NetworkProvider, useNetwork } from "@/components/shell/network-context"
import { DraftVault } from "@/lib/draft-vault"

const VaultContext = createContext<DraftVault | null>(null)

/**
 * Holds the console's per-network draft registries. It sits above the
 * `/<network>` segment so switching networks neither drops nor mixes drafts.
 * Nothing is persisted to storage.
 */
export function DraftVaultProvider({ children }: { children: ReactNode }) {
  const [vault] = useState(() => new DraftVault())
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (vault.anyDrafts()) event.preventDefault()
    }
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [vault])
  return <VaultContext.Provider value={vault}>{children}</VaultContext.Provider>
}

export function useDraftVault(): DraftVault {
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
          <ManagedListenerDraftProvider registry={drafts.managedListeners}>
            <AccessDraftProvider registry={drafts.access}>{children}</AccessDraftProvider>
          </ManagedListenerDraftProvider>
        </ConnectionsDraftProvider>
      </RoutingDraftProvider>
    </NetworkProvider>
  )
}

/** The network the current page was rendered for; actions are bound to it. */
export function useNetworkId(): string {
  return useNetwork().networkId
}
