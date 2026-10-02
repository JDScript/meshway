"use server"

import { getEndpointDiscoveries, type EndpointDiscoveriesSnapshot } from "@meshway/control-client"

import { controllerUrl, describeError, isCredentialRejected, type ControllerResult } from "@/lib/controller"
import { rejectActionSession } from "@/lib/session"
import type { ActionScope } from "@/lib/action-scope"
import { requireNetworkActionContext } from "@/lib/network-context"

/** Authenticated read only; the generated operation validates the response. */
export async function readEndpointDiscoveriesAction(scope: ActionScope): Promise<ControllerResult<EndpointDiscoveriesSnapshot>> {
  const { token, networkId } = await requireNetworkActionContext(scope)
  try {
    const data = await getEndpointDiscoveries(controllerUrl(), token, networkId)
    return { ok: true, data, fetchedAt: Date.now() }
  } catch (error) {
    if (!isCredentialRejected(error)) return { ok: false, error: describeError(error, token), fetchedAt: Date.now() }
  }
  return rejectActionSession()
}
