import "server-only"

import {
  ControlApiError,
  getConnectionRuntimes,
  getConnections,
  getMeshAccess,
  getNetwork,
  getOverview,
  getPolicy,
  getTags,
  listNodes,
  type ConnectionRuntimesSnapshot,
  type ConnectionsSnapshot,
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

export function controllerUrl(): string {
  return process.env.MESHWAY_CONTROLLER_URL ?? "http://127.0.0.1:8420"
}

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

export function fetchOverview(): Promise<ControllerResult<Overview>> {
  return authorizedRead(getOverview)
}

export function fetchNodes(): Promise<ControllerResult<NodeList>> {
  return authorizedRead(listNodes)
}

export function fetchPolicy(): Promise<ControllerResult<PolicySnapshot>> {
  return authorizedRead(getPolicy)
}

export function fetchConnections(): Promise<ControllerResult<ConnectionsSnapshot>> {
  return authorizedRead(getConnections)
}

/** Agent self-reports; a separate read so an outage never hides the saved plan. */
export function fetchConnectionRuntimes(): Promise<ControllerResult<ConnectionRuntimesSnapshot>> {
  return authorizedRead(getConnectionRuntimes)
}

/** Saved endpoint access rules (with their JSONC source, if stored). */
export function fetchMeshAccess(): Promise<ControllerResult<MeshAccessSnapshot>> {
  return authorizedRead(getMeshAccess)
}

/** Tag catalog and node tag assignments. */
export function fetchTags(): Promise<ControllerResult<TagSnapshot>> {
  return authorizedRead(getTags)
}

/** Persistent singleton network metadata and saved DNS intent. */
const readNetwork = cache(() => authorizedRead(getNetwork))

export function fetchNetwork(): Promise<ControllerResult<NetworkSnapshot>> {
  return readNetwork()
}
