"use server"

import { getEndpointDiscoveries, type EndpointDiscoveriesSnapshot } from "@meshway/control-client"

import { controllerUrl, describeError, isCredentialRejected, type ControllerResult } from "@/lib/controller"
import { rejectActionSession, requireActionAdminToken } from "@/lib/session"

/** Authenticated read only; the generated operation validates the response. */
export async function readEndpointDiscoveriesAction(): Promise<ControllerResult<EndpointDiscoveriesSnapshot>> {
  const token = await requireActionAdminToken()
  try {
    const data = await getEndpointDiscoveries(controllerUrl(), token)
    return { ok: true, data, fetchedAt: Date.now() }
  } catch (error) {
    if (!isCredentialRejected(error)) return { ok: false, error: describeError(error, token), fetchedAt: Date.now() }
  }
  return rejectActionSession()
}
