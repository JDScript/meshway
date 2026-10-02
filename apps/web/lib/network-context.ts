import "server-only"

import { listNetworks } from "@meshway/control-client"
import { zNetworkSnapshot } from "@meshway/control-client/schemas"
import { cookies } from "next/headers"
import { cache } from "react"
import { controllerUrl } from "@/lib/controller-config"
import { consoleCookieName } from "@/lib/cookie-name"
import type { ActionScope } from "@/lib/action-scope"
import { requireRenderedSignIn } from "@/lib/session"

export const NETWORK_COOKIE = consoleCookieName("meshway_network")

/** Every network action first checks that the request's cookie is the
 * sign-in its page was rendered for, then validates its explicit network.
 * Both come from the rendered workspace, never from a mutable selection
 * cookie. The credential may be a user session or the administrator token;
 * the controller authorizes every call against the session's live role in
 * this network inside the same transaction as the write. Network CAS
 * (expected revisions) stays the caller's own, independent check. */
export async function requireNetworkActionContext(scope: ActionScope) {
  const credential = await requireRenderedSignIn(typeof scope === "object" && scope !== null ? scope.signIn : undefined)
  const networkId: unknown = typeof scope === "object" && scope !== null ? scope.networkId : undefined
  return {
    token: credential.token,
    kind: credential.kind,
    baseUrl: controllerUrl(),
    networkId: zNetworkSnapshot.shape.network_id.parse(networkId),
  }
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
