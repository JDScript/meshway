// Pure encoding of the console session cookie (no Next.js imports, so it can
// be unit-tested). The cookie is HttpOnly and only ever read on the server.
//
//   v2.<kind>.<expires_at>.<session_id>.<credential>
//
// `kind` says which controller credential the cookie carries and therefore
// which authority the console may claim:
//   - "user":      a controller user session (`mws_…`), expiry set by the
//                  controller's absolute `expires_at`, never extended here.
//   - "bootstrap": the instance administrator token (compatibility entry),
//                  a fixed Web-side lifetime.
// `session_id` is a random Web-side identifier of this sign-in. It is not a
// credential; the console keys client-side state (drafts) by it, so a new
// sign-in never inherits another sign-in's unsaved work.

export type CredentialKind = "user" | "bootstrap"

export type ConsoleCredential = {
  kind: CredentialKind
  token: string
  /** Unix seconds; the cookie is unusable from this moment. */
  expiresAt: number
  sessionId: string
}

export type SessionState =
  | { kind: "none" }
  | { kind: "expired" }
  | { kind: "active"; credential: ConsoleCredential }

export const COOKIE_VERSION = "v2"
/** Fixed lifetime of an administrator-token console session. */
export const BOOTSTRAP_LIFETIME_SECONDS = 8 * 60 * 60

const USER_TOKEN = /^mws_[0-9a-f]{64}$/
const SESSION_ID = /^[0-9a-f]{32}$/
const VISIBLE_ASCII = /^[\x21-\x7E]+$/

/** Whether `token` has the shape of a controller user session. */
export function isUserSessionToken(token: string): boolean {
  return USER_TOKEN.test(token)
}

function validCredential(kind: CredentialKind, token: string): boolean {
  if (kind === "user") return isUserSessionToken(token)
  // An administrator token must never be (or be mistaken for) a user session.
  return token.length > 0 && token.length <= 1024 && VISIBLE_ASCII.test(token) && !token.startsWith("mws_")
}

export function encodeSessionCookie(credential: ConsoleCredential): string {
  if (!validCredential(credential.kind, credential.token) || !SESSION_ID.test(credential.sessionId)) {
    throw new Error("Invalid console credential")
  }
  return [COOKIE_VERSION, credential.kind, String(credential.expiresAt), credential.sessionId, credential.token].join(".")
}

/** Anything that is not a well-formed, unexpired v2 cookie is "expired" (and gets discarded). */
export function parseSessionCookie(value: string, nowSeconds: number): SessionState {
  const [version, kind, expiresAt, sessionId, ...rest] = value.split(".")
  const token = rest.join(".")
  const expiresAtSeconds = Number(expiresAt)
  if (
    version !== COOKIE_VERSION ||
    (kind !== "user" && kind !== "bootstrap") ||
    !Number.isSafeInteger(expiresAtSeconds) ||
    sessionId === undefined ||
    !SESSION_ID.test(sessionId) ||
    !validCredential(kind, token)
  ) {
    return { kind: "expired" }
  }
  if (expiresAtSeconds <= nowSeconds) return { kind: "expired" }
  return { kind: "active", credential: { kind, token, expiresAt: expiresAtSeconds, sessionId } }
}

/**
 * Cookie Max-Age for a user session: whatever the controller granted, never
 * more. Returns null when the controller's expiry is already in the past.
 */
export function userSessionMaxAge(expiresAt: number, nowSeconds: number): number | null {
  if (!Number.isSafeInteger(expiresAt)) return null
  const remaining = expiresAt - nowSeconds
  return remaining > 0 ? remaining : null
}
