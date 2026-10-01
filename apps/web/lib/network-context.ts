import "server-only"

import { listNetworks } from "@meshway/control-client"
import { zNetworkSnapshot } from "@meshway/control-client/schemas"
import { cookies } from "next/headers"
import { cache } from "react"
import { controllerUrl } from "@/lib/controller-config"
import { consoleCookieName } from "@/lib/cookie-name"
import { requireActionAdminToken } from "@/lib/session"

export const NETWORK_COOKIE = consoleCookieName("meshway_network")

/** Every network action authenticates first, then validates its explicit scope.
 * Scope comes from the rendered workspace, never from a mutable selection
 * cookie. Existence and authorization are checked again by the controller. */
export async function requireNetworkActionContext(networkId: string) {
  const token = await requireActionAdminToken()
  return { token, baseUrl: controllerUrl(), networkId: zNetworkSnapshot.shape.network_id.parse(networkId) }
}

export const selectedNetworkId = cache(async (token: string): Promise<string> => {
  const networks = await listNetworks(controllerUrl(), token)
  const selected = (await cookies()).get(NETWORK_COOKIE)?.value
  const network = selected
    ? networks.networks.find(network => network.network_id === selected)
    : networks.networks[0]
  if (!network) throw new Error("所选网络不存在，请重新选择网络。")
  return network.network_id
})
