import "server-only"

import {
  ControlApiError,
  getConnectionRuntimes,
  getConnections,
  getEndpointDiscoveries,
  getManagedListeners,
  getMeshAccess,
  getNetwork,
  getOverview,
  getPolicy,
  getTags,
  listAccountInvitations,
  listAccountMembers,
  listNodeOwnership,
  listNodes,
  listNetworks,
  type AccountInvitations,
  type AccountMembers,
  type NodeOwnershipList,
  type ConnectionRuntimesSnapshot,
  type ConnectionsSnapshot,
  type EndpointDiscoveriesSnapshot,
  type ManagedListenersSnapshot,
  type MeshAccessSnapshot,
  type NodeList,
  type Overview,
  type PolicySnapshot,
  type TagSnapshot,
} from "@meshway/control-client"
import { redirect } from "next/navigation"
import { connection } from "next/server"
import { cache } from "react"

import { EXPIRED_LOGIN_PATH, requireConsoleToken } from "@/lib/session"
import { controllerUrl } from "@/lib/controller-config"

export { controllerUrl } from "@/lib/controller-config"

export type ControllerResult<T> =
  | { ok: true; data: T; fetchedAt: number }
  | { ok: false; error: string; fetchedAt: number }

/**
 * The controller rejected the console credential itself (HTTP 401): expired,
 * logged out, revoked or reset. Only this ends the console session.
 */
export function isCredentialRejected(error: unknown): boolean {
  return error instanceof ControlApiError && error.status === 401
}

/**
 * The credential is valid but lacks the permission (HTTP 403), e.g. a Member
 * writing configuration. The session stays; the caller shows the error.
 */
export function isPermissionDenied(error: unknown): boolean {
  return error instanceof ControlApiError && error.status === 403
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
  if (isPermissionDenied(error)) messages.unshift("当前账户没有此操作的权限")
  const text = messages.length > 0 ? messages.join(": ") : "未知错误"
  // Bearer values are never meant to appear in SDK errors; redact defensively.
  return secret ? text.replaceAll(secret, "[已隐藏]") : text
}

// Each call hits the controller at request time; nothing is cached. A missing
// or expired session redirects to login; a 401 from the controller redirects
// to login where the stale cookie is discarded. Any other failure (including
// a 403 permission refusal) is shown as a controller error rather than
// treated as an authentication failure.
async function authorizedRead<T>(
  call: (baseUrl: string, token: string) => Promise<T>
): Promise<ControllerResult<T>> {
  await connection()
  const token = await requireConsoleToken()
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

// Every network read names its network explicitly: pages take it from the
// `/<network_id>` URL segment (see lib/network-scope.ts), never from the
// selection cookie, so a page always shows the network its URL names.
function networkRead<T>(
  networkId: string,
  call: (baseUrl: string, token: string, networkId: string) => Promise<T>
): Promise<ControllerResult<T>> {
  return authorizedRead((base, token) => call(base, token, networkId))
}

export function fetchOverview(networkId: string): Promise<ControllerResult<Overview>> {
  return networkRead(networkId, getOverview)
}

export function fetchNodes(networkId: string): Promise<ControllerResult<NodeList>> {
  return networkRead(networkId, listNodes)
}

export function fetchPolicy(networkId: string): Promise<ControllerResult<PolicySnapshot>> {
  return networkRead(networkId, getPolicy)
}

export function fetchConnections(networkId: string): Promise<ControllerResult<ConnectionsSnapshot>> {
  return networkRead(networkId, getConnections)
}

/** Agent self-reports; a separate read so an outage never hides the saved plan. */
export function fetchConnectionRuntimes(networkId: string): Promise<ControllerResult<ConnectionRuntimesSnapshot>> {
  return networkRead(networkId, getConnectionRuntimes)
}

export function fetchEndpointDiscoveries(networkId: string): Promise<ControllerResult<EndpointDiscoveriesSnapshot>> {
  return networkRead(networkId, getEndpointDiscoveries)
}

/** Platform-managed listener intent and public material status (never secrets). */
export function fetchManagedListeners(networkId: string): Promise<ControllerResult<ManagedListenersSnapshot>> {
  return networkRead(networkId, getManagedListeners)
}

/** Saved endpoint access rules (with their JSONC source, if stored). */
export function fetchMeshAccess(networkId: string): Promise<ControllerResult<MeshAccessSnapshot>> {
  return networkRead(networkId, getMeshAccess)
}

/** Tag catalog and node tag assignments. */
export function fetchTags(networkId: string): Promise<ControllerResult<TagSnapshot>> {
  return networkRead(networkId, getTags)
}

/** Live memberships of this network (any member may read). Shared within one request. */
export const fetchMembers = cache((networkId: string): Promise<ControllerResult<AccountMembers>> => networkRead(networkId, listAccountMembers))

/** Invitation metadata (never secrets); Owners see all, Admins the Member ones, Members are refused (403). */
export function fetchInvitations(networkId: string): Promise<ControllerResult<AccountInvitations>> {
  return networkRead(networkId, listAccountInvitations)
}

/** Which membership owns each node; null is a legacy node nobody has claimed. Shared within one request. */
export const fetchNodeOwnership = cache((networkId: string): Promise<ControllerResult<NodeOwnershipList>> => networkRead(networkId, listNodeOwnership))

/** Network metadata and saved DNS intent, shared by the shell and pages within one request. */
export const fetchNetwork = cache((networkId: string) => networkRead(networkId, getNetwork))

/** Global: every network this credential may see (a user: its live memberships). Shared within one request. */
export const fetchNetworks = cache(() => authorizedRead(listNetworks))
