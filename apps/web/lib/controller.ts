import "server-only"

import {
  ControlApiError,
  getConnectionRuntimes,
  getConnections,
  getEndpointDiscoveries,
  getMeshAccess,
  getNetwork,
  getOverview,
  getPolicy,
  getTags,
  listNodes,
  listNetworks,
  type NetworkList,
  type ConnectionRuntimesSnapshot,
  type ConnectionsSnapshot,
  type EndpointDiscoveriesSnapshot,
  type MeshAccessSnapshot,
  type NetworkSnapshot,
  type NodeList,
  type Overview,
  type PolicySnapshot,
  type TagSnapshot,
} from "@meshway/control-client"
import { redirect } from "next/navigation"
import { connection } from "next/server"
import { cache } from "react"

import { EXPIRED_LOGIN_PATH, requireAdminToken } from "@/lib/session"
import { selectedNetworkId } from "@/lib/network-context"
import { controllerUrl } from "@/lib/controller-config"

export { controllerUrl } from "@/lib/controller-config"

export type ControllerResult<T> =
  | { ok: true; data: T; fetchedAt: number }
  | { ok: false; error: string; fetchedAt: number }

/** The controller rejected the administrator credential itself. */
export function isCredentialRejected(error: unknown): boolean {
  return error instanceof ControlApiError && error.status === 401
}

export function describeError(error: unknown, secret?: string): string {
  // Node's fetch reports the socket-level reason (ECONNREFUSED, ENOTFOUND…)
  // somewhere down the `cause` chain, possibly under SDK wrapper errors.
  const messages: string[] = []
  let current: unknown = error
  while (current instanceof Error && messages.length < 5) {
    const message = current.message
    // SDK wrappers already embed their cause's message; skip repeats.
    if (!messages.some((seen) => seen.includes(message))) messages.push(message)
    current = current.cause
  }
  if (error instanceof ControlApiError && !messages.some((m) => m.includes(String(error.status)))) {
    messages.unshift(`HTTP ${error.status}`)
  }
  const text = messages.length > 0 ? messages.join(": ") : "未知错误"
  // Bearer values are never meant to appear in SDK errors; redact defensively.
  return secret ? text.replaceAll(secret, "[已隐藏]") : text
}

// Each call hits the controller at request time; nothing is cached. A missing
// or expired session redirects to login; a 401 from the controller redirects
// to login where the stale cookie is discarded. Any other failure is shown as
// a controller error rather than treated as an authentication failure.
async function authorizedRead<T>(
  call: (baseUrl: string, token: string) => Promise<T>
): Promise<ControllerResult<T>> {
  await connection()
  const token = await requireAdminToken()
  try {
    const data = await call(controllerUrl(), token)
    return { ok: true, data, fetchedAt: Date.now() }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return { ok: false, error: describeError(error, token), fetchedAt: Date.now() }
    }
  }
  redirect(EXPIRED_LOGIN_PATH)
}

function authorizedNetworkRead<T>(call: (baseUrl: string, token: string, networkId: string) => Promise<T>): Promise<ControllerResult<T>> {
  return authorizedRead(async (base, token) => call(base, token, await selectedNetworkId(token)))
}

export function fetchOverview(): Promise<ControllerResult<Overview>> {
  return authorizedNetworkRead(getOverview)
}

export function fetchNodes(): Promise<ControllerResult<NodeList>> {
  return authorizedNetworkRead(listNodes)
}

export function fetchPolicy(): Promise<ControllerResult<PolicySnapshot>> {
  return authorizedNetworkRead(getPolicy)
}

export function fetchConnections(): Promise<ControllerResult<ConnectionsSnapshot>> {
  return authorizedNetworkRead(getConnections)
}

/** Agent self-reports; a separate read so an outage never hides the saved plan. */
export function fetchConnectionRuntimes(): Promise<ControllerResult<ConnectionRuntimesSnapshot>> {
  return authorizedNetworkRead(getConnectionRuntimes)
}

export function fetchEndpointDiscoveries(): Promise<ControllerResult<EndpointDiscoveriesSnapshot>> {
  return authorizedNetworkRead(getEndpointDiscoveries)
}

/** Saved endpoint access rules (with their JSONC source, if stored). */
export function fetchMeshAccess(): Promise<ControllerResult<MeshAccessSnapshot>> {
  return authorizedNetworkRead(getMeshAccess)
}

/** Tag catalog and node tag assignments. */
export function fetchTags(): Promise<ControllerResult<TagSnapshot>> {
  return authorizedNetworkRead(getTags)
}

/** Persistent singleton network metadata and saved DNS intent. */
const readNetwork = cache(() => authorizedNetworkRead(getNetwork))

export function fetchNetwork(): Promise<ControllerResult<NetworkSnapshot>> {
  return readNetwork()
}

export function fetchNetworks(): Promise<ControllerResult<NetworkList>> {
  return authorizedRead(listNetworks)
}
