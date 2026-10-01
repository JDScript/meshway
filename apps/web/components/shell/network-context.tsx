"use client"

import { createContext, useContext, useMemo, type ReactNode } from "react"

import { consoleRoutes, type ConsoleRoutes } from "@/lib/routes"

export type NetworkContextValue = {
  networkId: string
  /** Display name, when the network record could be read. */
  name: string | null
  routes: ConsoleRoutes
}

const NetworkContext = createContext<NetworkContextValue | null>(null)

export function NetworkProvider({
  networkId,
  name,
  children,
}: {
  networkId: string
  name: string | null
  children: ReactNode
}) {
  const value = useMemo(() => ({ networkId, name, routes: consoleRoutes(networkId) }), [networkId, name])
  return <NetworkContext.Provider value={value}>{children}</NetworkContext.Provider>
}

export function useNetwork(): NetworkContextValue {
  const value = useContext(NetworkContext)
  if (!value) throw new Error("useNetwork must be used inside a network-scoped console page")
  return value
}

/** Path builders for the network the current page belongs to. */
export function useRoutes(): ConsoleRoutes {
  return useNetwork().routes
}
