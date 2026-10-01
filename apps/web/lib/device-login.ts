import { zAuthorizeDeviceRequest } from "@meshway/control-client/schemas"
import type { AuthorizeDeviceRequest } from "@meshway/control-client"

export type DeviceAuthorization = { request: AuthorizeDeviceRequest; state: string }
type Params = Record<string, string | string[] | undefined>
const keys = ["redirect_uri", "state", "code_challenge", "name", "agent_version"]

export function parseDeviceAuthorization(params: Params): DeviceAuthorization | null {
  if (keys.some((key) => typeof params[key] !== "string")) return null
  const state = params.state
  if (typeof state !== "string" || !state.length || [...state].length > 256 || [...state].some(char => char.charCodeAt(0) < 32 || (char.charCodeAt(0) >= 127 && char.charCodeAt(0) <= 159))) return null
  const result = zAuthorizeDeviceRequest.safeParse({
    redirect_uri: params.redirect_uri, code_challenge: params.code_challenge,
    name: params.name, agent_version: params.agent_version,
  })
  return result.success ? { request: result.data, state } : null
}

export function authorizationPath(value: DeviceAuthorization): string {
  return `/authorize-device?${new URLSearchParams({ ...value.request, state: value.state })}`
}

/** Only the validated, canonical device entry can be a login return target. */
export function safeLoginReturn(value: unknown): string {
  if (typeof value !== "string" || value.length > 8192 || !value.startsWith("/authorize-device?")) return "/"
  const query = value.slice("/authorize-device?".length)
  const params: Params = {}
  for (const [key, item] of new URLSearchParams(query)) {
    if (params[key] !== undefined || !keys.includes(key)) return "/"
    params[key] = item
  }
  const parsed = parseDeviceAuthorization(params)
  return parsed ? authorizationPath(parsed) : "/"
}

export function deviceCallback(value: DeviceAuthorization, result: { code: string } | { error: "access_denied" }): string {
  // Revalidate even when callers have a typed value; no arbitrary redirect.
  const checked = parseDeviceAuthorization({ ...value.request, state: value.state })
  if (!checked) throw new Error("Invalid device authorization")
  const url = new URL(checked.request.redirect_uri)
  url.search = new URLSearchParams({ ...result, state: checked.state }).toString()
  return url.toString()
}
