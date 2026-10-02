import type { AuthorizeDeviceRequest } from "@meshway/control-client"
import { zAuthorizeDeviceRequest } from "@meshway/control-client/schemas"

// The App's half of the device sign-in (docs/device-login.md): it opens
// `/authorize-device?redirect_uri&state&code_challenge&name&agent_version`
// and expects `redirect_uri?code&state` or `redirect_uri?error=access_denied&state`.
// The same contract as the earlier console (apps/web/lib/device-login.ts).

export type DeviceRequest = {
  request: AuthorizeDeviceRequest
  /** Opaque to the controller; only echoed back to the App. */
  state: string
}

export const DEVICE_PATH = "/authorize-device"

const KEYS = [
  "redirect_uri",
  "state",
  "code_challenge",
  "name",
  "agent_version",
] as const

const MAX_STATE = 256

function validState(state: string): boolean {
  const chars = [...state]
  return (
    chars.length > 0 &&
    chars.length <= MAX_STATE &&
    !chars.some((char) => {
      const code = char.charCodeAt(0)
      return code < 32 || (code >= 127 && code <= 159)
    })
  )
}

/**
 * Reads the raw query string (not the router's parsed search, which turns
 * values such as `1.0` into numbers). Every field must appear exactly once;
 * other keys are ignored. Null when the request is not one the App makes.
 */
export function parseDeviceRequest(query: string): DeviceRequest | null {
  const params = new URLSearchParams(query)
  const value: Partial<Record<(typeof KEYS)[number], string>> = {}
  for (const key of KEYS) {
    const all = params.getAll(key)
    if (all.length !== 1) return null
    value[key] = all[0]
  }
  const state = value.state
  if (state === undefined || !validState(state)) return null
  const request = zAuthorizeDeviceRequest.safeParse({
    redirect_uri: value.redirect_uri,
    code_challenge: value.code_challenge,
    name: value.name,
    agent_version: value.agent_version,
  })
  return request.success ? { request: request.data, state } : null
}

/** The canonical page path for a request: the sign-in returns here. */
export function devicePath({ request, state }: DeviceRequest): string {
  const query = new URLSearchParams({
    redirect_uri: request.redirect_uri,
    state,
    code_challenge: request.code_challenge,
    name: request.name,
    agent_version: request.agent_version,
  })
  return `${DEVICE_PATH}?${query}`
}

export type DeviceDecision = { code: string } | { error: "access_denied" }

/** Where the App receives the decision; revalidated so no other target is reachable. */
export function deviceCallback(
  value: DeviceRequest,
  decision: DeviceDecision
): string {
  const checked = parseDeviceRequest(devicePath(value).split("?")[1] ?? "")
  if (!checked) throw new Error("invalid device request")
  const url = new URL(checked.request.redirect_uri)
  url.search = new URLSearchParams(
    "code" in decision
      ? { code: decision.code, state: checked.state }
      : { error: decision.error, state: checked.state }
  ).toString()
  return url.toString()
}
