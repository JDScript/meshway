import "server-only"

import { randomBytes } from "node:crypto"
import { cookies, headers } from "next/headers"
import { redirect } from "next/navigation"

import { isSignInKey, SESSION_CHANGED_PATH } from "@/lib/action-scope"
import { consoleCookieName, cookieNamespace } from "@/lib/cookie-name"
import {
  activeCookieName,
  BOOTSTRAP_LIFETIME_SECONDS,
  credentialCookieName,
  credentialCookieSessionId,
  encodeSessionCookie,
  parseActivePointer,
  parseSessionCookie,
  signInKeyOf,
  userSessionMaxAge,
  type ConsoleCredential,
  type SessionState,
} from "@/lib/session-cookie"

export type { ConsoleCredential, CredentialKind, SessionState } from "@/lib/session-cookie"

// One active console sign-in per browser: either a controller user session
// or the instance administrator token (compatibility entry). Each sign-in's
// credential lives in its own HttpOnly, SameSite=Strict cookie, and a
// credential-free pointer names the active one (lib/session-cookie.ts). The
// credential cookie carries its own absolute expiry, so the lifetime is
// enforced here and not only by the browser's Max-Age handling.
// Earlier single-cookie layouts (never a valid sign-in now); removed on sign-in and cleanup.
const LEGACY_COOKIE_NAMES = ["meshway_console_session", "meshway_admin_session"]

export const LOGIN_PATH = "/login"
export const EXPIRED_LOGIN_PATH = "/login?reason=expired"

const now = () => Math.floor(Date.now() / 1000)

type StoredSession = { state: SessionState; sessionId: string | null }

/** The active pointer and its own credential cookie, from this request's cookies. */
async function readStored(): Promise<StoredSession> {
  const jar = await cookies()
  const namespace = cookieNamespace()
  const sessionId = parseActivePointer(jar.get(activeCookieName(namespace))?.value)
  const legacy = LEGACY_COOKIE_NAMES.some((name) => jar.get(consoleCookieName(name)))
  // No credential for the pointer: signed out ("expired" only so the login page tidies old layouts).
  const credential = sessionId === null ? undefined : jar.get(credentialCookieName(sessionId, namespace))
  if (sessionId === null || !credential) return { state: legacy ? { kind: "expired" } : { kind: "none" }, sessionId }
  const state = parseSessionCookie(credential.value, now())
  // The pointer, the cookie name and the envelope must all name the same sign-in.
  if (state.kind === "active" && state.credential.sessionId !== sessionId) return { state: { kind: "expired" }, sessionId }
  return { state, sessionId }
}

export async function readSession(): Promise<SessionState> {
  return (await readStored()).state
}

/** For Server Component renders: redirects when no usable session exists. */
export async function requireConsoleCredential(): Promise<ConsoleCredential> {
  const session = await readSession()
  if (session.kind === "active") return session.credential
  redirect(session.kind === "expired" ? EXPIRED_LOGIN_PATH : LOGIN_PATH)
}

/** Server Component renders: the console credential's bearer, whatever its kind. */
export async function requireConsoleToken(): Promise<string> {
  return (await requireConsoleCredential()).token
}

/** @deprecated Historic name; any console credential (user or administrator token). */
export const requireAdminToken = requireConsoleToken

/**
 * For Server Actions without a rendered sign-in (sign-in flows only). Never
 * clears the cookie: the request may be late and the browser may hold a
 * newer sign-in by now. An unusable cookie is cleared by the login page,
 * which checks the cookie the browser sends at that time.
 */
export async function requireActionCredential(): Promise<ConsoleCredential> {
  const session = await readSession()
  if (session.kind === "active") return session.credential
  redirect(session.kind === "expired" ? EXPIRED_LOGIN_PATH : LOGIN_PATH)
}

/**
 * For every Server Action with effects (and its reads): the cookie this
 * request carries must be the sign-in the page was rendered for. A missing
 * or different sign-in is refused before anything reaches the controller,
 * without touching the (possibly newer, valid) cookie.
 */
export async function requireRenderedSignIn(expected: unknown): Promise<ConsoleCredential> {
  const session = await readSession()
  if (session.kind !== "active") redirect(session.kind === "expired" ? EXPIRED_LOGIN_PATH : LOGIN_PATH)
  if (!isSignInKey(expected) || signInKeyOf(session.credential) !== expected) redirect(SESSION_CHANGED_PATH)
  return session.credential
}

/** Whether this request's cookie is the rendered sign-in `expected`. */
export async function isRenderedSignIn(expected: unknown): Promise<boolean> {
  const session = await readSession()
  return session.kind === "active" && isSignInKey(expected) && signInKeyOf(session.credential) === expected
}

/**
 * For Server Actions whose credential the controller rejected (HTTP 401).
 * The cookie is not cleared here: this response may arrive after another
 * sign-in replaced it. The login page verifies whichever cookie the browser
 * then sends and clears it only if the controller rejects that one.
 */
export async function rejectActionSession(): Promise<never> {
  redirect(EXPIRED_LOGIN_PATH)
}

/**
 * MESHWAY_WEB_COOKIE_SECURE: "true" always sets Secure, "false" never does,
 * unset or "auto" sets it when the browser reached the console over HTTPS.
 * Any other value fails closed to Secure.
 */
export async function secureCookie(): Promise<boolean> {
  const setting = process.env.MESHWAY_WEB_COOKIE_SECURE?.trim().toLowerCase()
  if (setting === "false") return false
  if (setting !== undefined && setting !== "" && setting !== "auto") return true
  const requestHeaders = await headers()
  const origin = requestHeaders.get("origin")
  if (origin) {
    try {
      return new URL(origin).protocol === "https:"
    } catch {
      return true
    }
  }
  return requestHeaders.get("x-forwarded-proto")?.split(",")[0]?.trim() === "https"
}

async function writeCookie(name: string, value: string, maxAge: number) {
  const secure = await secureCookie()
  ;(await cookies()).set(name, value, {
    httpOnly: true,
    sameSite: "strict",
    secure,
    path: "/",
    maxAge,
    priority: "high",
  })
}

async function start(credential: Omit<ConsoleCredential, "sessionId">, maxAge: number) {
  const jar = await cookies()
  const namespace = cookieNamespace()
  const sessionId = randomBytes(16).toString("hex")
  const value = encodeSessionCookie({ ...credential, sessionId })
  await writeCookie(credentialCookieName(sessionId, namespace), value, maxAge)
  // Only a sign-in moves the pointer; it lives no longer than its credential.
  await writeCookie(activeCookieName(namespace), sessionId, maxAge)
  // Bounded cleanup: every other credential of this namespace, and old layouts.
  for (const cookie of jar.getAll()) {
    const other = credentialCookieSessionId(cookie.name, namespace)
    if (other !== null && other !== sessionId) await writeCookie(cookie.name, "", 0)
  }
  for (const name of LEGACY_COOKIE_NAMES) if (jar.get(consoleCookieName(name))) await writeCookie(consoleCookieName(name), "", 0)
}

/** Server Actions only. The administrator token, after the controller proved it. */
export async function startBootstrapSession(token: string) {
  await start({ kind: "bootstrap", token, expiresAt: now() + BOOTSTRAP_LIFETIME_SECONDS }, BOOTSTRAP_LIFETIME_SECONDS)
}

/**
 * Server Actions only. A user session exactly as long as the controller
 * granted it (absolute expiry, no sliding renewal). False if already expired.
 */
export async function startUserSession(token: string, expiresAt: number): Promise<boolean> {
  const maxAge = userSessionMaxAge(expiresAt, now())
  if (maxAge === null) return false
  await start({ kind: "user", token, expiresAt }, maxAge)
  return true
}

/**
 * Server Actions only. Deletes the credential cookie of exactly the active
 * sign-in this request carries (and old layouts). Never the pointer, never
 * another sign-in's credential: if this response is delayed past a newer
 * sign-in, it only removes the old, already replaced credential.
 */
export async function endSession() {
  const jar = await cookies()
  const { sessionId } = await readStored()
  if (sessionId !== null) await writeCookie(credentialCookieName(sessionId, cookieNamespace()), "", 0)
  for (const name of LEGACY_COOKIE_NAMES) if (jar.get(consoleCookieName(name))) await writeCookie(consoleCookieName(name), "", 0)
}

/**
 * Server Actions only. Deletes the credential cookie(s) of the sign-in `key`
 * that this request carries, wherever the pointer is; nothing else.
 */
export async function endSignIn(key: unknown): Promise<void> {
  if (!isSignInKey(key)) return
  const jar = await cookies()
  const namespace = cookieNamespace()
  for (const cookie of jar.getAll()) {
    if (credentialCookieSessionId(cookie.name, namespace) === null) continue
    const parsed = parseSessionCookie(cookie.value, Number.MIN_SAFE_INTEGER)
    if (parsed.kind === "active" && signInKeyOf(parsed.credential) === key) await writeCookie(cookie.name, "", 0)
  }
}

/** Non-secret key of the current sign-in ("none" / "expired" without one), for fencing and keying client state. */
export async function sessionKey(): Promise<string> {
  const session = await readSession()
  return session.kind === "active" ? signInKeyOf(session.credential) : session.kind
}
