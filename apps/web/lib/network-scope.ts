import "server-only"

import type { NetworkList, NetworkSnapshot } from "@meshway/control-client"
import { cookies } from "next/headers"
import { notFound } from "next/navigation"

import { fetchNetworks, type ControllerResult } from "@/lib/controller"
import { NETWORK_COOKIE } from "@/lib/network-context"
import { consoleRoutes, isNetworkId, type ConsoleRoutes } from "@/lib/routes"

export type NetworkScope = {
  networkId: string
  routes: ConsoleRoutes
  /** This network's record from the list read, or why the list could not be read. */
  network: { ok: true; data: NetworkSnapshot } | { ok: false; error: string }
  networks: ControllerResult<NetworkList>
}

/**
 * Resolves a page's `/<network_id>` segment. The URL is the only source of
 * the page's network: the selection cookie never retargets an open page.
 * An ID the controller does not list is a 404, never a fallback to another
 * network. If the list itself is unavailable the page still renders its
 * own reads, which report their failures.
 */
export async function requireNetwork(params: Promise<{ network: string }>): Promise<NetworkScope> {
  const { network: segment } = await params
  if (!isNetworkId(segment)) notFound()
  const networks = await fetchNetworks()
  const routes = consoleRoutes(segment)
  if (!networks.ok) return { networkId: segment, routes, networks, network: { ok: false, error: networks.error } }
  const network = networks.data.networks.find((candidate) => candidate.network_id === segment)
  if (!network) notFound()
  return { networkId: segment, routes, networks, network: { ok: true, data: network } }
}

/**
 * Where `/` and pre-network URLs land: the last network chosen in the
 * switcher if it still exists, else the controller's first network.
 */
export async function landingNetwork(): Promise<{ ok: true; networkId: string } | { ok: false; error: string }> {
  const networks = await fetchNetworks()
  if (!networks.ok) return { ok: false, error: networks.error }
  const selected = (await cookies()).get(NETWORK_COOKIE)?.value
  const network =
    networks.data.networks.find((candidate) => candidate.network_id === selected) ?? networks.data.networks[0]
  if (!network) return { ok: false, error: "Controller 没有任何网络。" }
  return { ok: true, networkId: network.network_id }
}
