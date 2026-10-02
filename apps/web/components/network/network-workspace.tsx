"use client"

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react"

import { AccessDraftProvider } from "@/components/access/draft-store"
import { ConnectionsDraftProvider } from "@/components/connections/draft-store"
import { ManagedListenerDraftProvider } from "@/components/managed-listeners/draft-store"
import { RoutingDraftProvider } from "@/components/routing/draft-store"
import { NetworkProvider, useNetwork } from "@/components/shell/network-context"
import type { ActionScope, SignInKey } from "@/lib/action-scope"
import { DraftVault } from "@/lib/draft-vault"

const VaultContext = createContext<DraftVault | null>(null)
const SignInContext = createContext<SignInKey | null>(null)

/**
 * Holds the console's per-network draft registries for one sign-in (the
 * console layout keys it by `signIn`). It sits above the `/<network>` segment
 * so switching networks neither drops nor mixes drafts. Nothing is persisted
 * to storage. Every action from here carries `signIn`, and the server refuses
 * it once the browser's cookie is another sign-in.
 */
export function DraftVaultProvider({ signIn, children }: { signIn: SignInKey; children: ReactNode }) {
  const [vault] = useState(() => new DraftVault())
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (vault.anyDrafts()) event.preventDefault()
    }
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [vault])
  return (
    <SignInContext.Provider value={signIn}>
      <VaultContext.Provider value={vault}>{children}</VaultContext.Provider>
    </SignInContext.Provider>
  )
}

/** The sign-in this console was rendered for; passed to global actions. */
export function useSignIn(): SignInKey {
  const signIn = useContext(SignInContext)
  if (signIn === null) throw new Error("Sign-in is unavailable")
  return signIn
}

/** First argument of every network action: the rendered network and sign-in. */
export function useActionScope(): ActionScope {
  const networkId = useNetwork().networkId
  const signIn = useSignIn()
  return useMemo(() => ({ networkId, signIn }), [networkId, signIn])
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
