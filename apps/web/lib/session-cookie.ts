import { createHash } from "node:crypto"

// Pure encoding of the console session cookies (no Next.js imports, so it
// can be unit-tested). They are HttpOnly and only ever read on the server.
//
// Each sign-in keeps its credential in its own cookie, named after its
// random Web sign-in ID:
//
//   meshway_console_credential_<session_id>[_<namespace>]
//     = v3.<kind>.<expires_at>.<session_id>.<credential>
//
// and a credential-free pointer names the active one:
//
//   meshway_console_active[_<namespace>] = <session_id>
//
// Only a sign-in writes the pointer. Sign-out and cleanup delete only the
// credential cookie of the sign-in they handled, never the pointer, so a
// delayed deletion for an older sign-in cannot remove a newer sign-in's
// credential (it has another name). A pointer without its credential means
// signed out.
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

export const COOKIE_VERSION = "v3"
/** Fixed lifetime of an administrator-token console session. */
export const BOOTSTRAP_LIFETIME_SECONDS = 8 * 60 * 60

const CREDENTIAL_COOKIE = "meshway_console_credential_"
const ACTIVE_COOKIE = "meshway_console_active"

const withNamespace = (name: string, namespace: string | undefined) => (namespace ? `${name}_${namespace}` : name)

export function activeCookieName(namespace: string | undefined): string {
  return withNamespace(ACTIVE_COOKIE, namespace)
}

export function credentialCookieName(sessionId: string, namespace: string | undefined): string {
  if (!SESSION_ID.test(sessionId)) throw new Error("Invalid sign-in ID")
  return withNamespace(`${CREDENTIAL_COOKIE}${sessionId}`, namespace)
}

/** The sign-in ID of a credential cookie of exactly this namespace, else null (other namespaces, other cookies). */
export function credentialCookieSessionId(name: string, namespace: string | undefined): string | null {
  const suffix = namespace ? `_${namespace}` : ""
  if (!name.startsWith(CREDENTIAL_COOKIE) || !name.endsWith(suffix)) return null
  const sessionId = name.slice(CREDENTIAL_COOKIE.length, name.length - suffix.length)
  return SESSION_ID.test(sessionId) ? sessionId : null
}

/** The active pointer's sign-in ID, if well-formed. */
export function parseActivePointer(value: string | undefined): string | null {
  return value !== undefined && SESSION_ID.test(value) ? value : null
}

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

/**
 * The rendered sign-in fence key. The cookie envelope is not signed, so its
 * kind / session ID alone could be edited; the marker therefore commits to
 * the high-entropy credential itself (domain-separated SHA-256, truncated).
 * It is not reversible into the credential and is not an authorization:
 * the controller still authenticates the credential on every call. A new
 * sign-in (even of the same user) has a new credential, hence a new key.
 */
export function signInKeyOf(credential: ConsoleCredential): string {
  const marker = createHash("sha256")
    .update("meshway-web-sign-in-fence-v1\0")
    .update(credential.kind)
    .update("\0")
    .update(credential.sessionId)
    .update("\0")
    .update(credential.token)
    .digest("hex")
    .slice(0, 32)
  return `${credential.kind}:${marker}`
}

