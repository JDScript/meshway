import "server-only"

import { randomBytes } from "node:crypto"
import { cookies, headers } from "next/headers"
import { redirect } from "next/navigation"

import { isSignInKey, SESSION_CHANGED_PATH } from "@/lib/action-scope"
import { consoleCookieName } from "@/lib/cookie-name"
import {
  BOOTSTRAP_LIFETIME_SECONDS,
  encodeSessionCookie,
  parseSessionCookie,
  signInKeyOf,
  userSessionMaxAge,
  type ConsoleCredential,
  type SessionState,
} from "@/lib/session-cookie"

export type { ConsoleCredential, CredentialKind, SessionState } from "@/lib/session-cookie"

// One console sign-in per browser: either a controller user session or the
// instance administrator token (compatibility entry). The credential lives in
// an HttpOnly, SameSite=Strict cookie and is only read on the server. The
// cookie carries its own absolute expiry, so the lifetime is enforced here
// and not only by the browser's Max-Age handling.
const COOKIE_NAME = consoleCookieName("meshway_console_session")
// Pre-account cookie (administrator token only); dropped on any sign-in/out.
const LEGACY_COOKIE_NAME = consoleCookieName("meshway_admin_session")

export const LOGIN_PATH = "/login"
export const EXPIRED_LOGIN_PATH = "/login?reason=expired"

const now = () => Math.floor(Date.now() / 1000)

export async function readSession(): Promise<SessionState> {
  const jar = await cookies()
  const cookie = jar.get(COOKIE_NAME)
  if (!cookie) return jar.get(LEGACY_COOKIE_NAME) ? { kind: "expired" } : { kind: "none" }
  return parseSessionCookie(cookie.value, now())
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
  const value = encodeSessionCookie({ ...credential, sessionId: randomBytes(16).toString("hex") })
  await writeCookie(COOKIE_NAME, value, maxAge)
  if ((await cookies()).get(LEGACY_COOKIE_NAME)) await writeCookie(LEGACY_COOKIE_NAME, "", 0)
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

/** Server Actions only, for the cookie of this very request (see `endSessionIfCurrent`). */
export async function endSession() {
  const jar = await cookies()
  await writeCookie(COOKIE_NAME, "", 0)
  if (jar.get(LEGACY_COOKIE_NAME)) await writeCookie(LEGACY_COOKIE_NAME, "", 0)
}

/**
 * Server Actions only. Clears the cookie only when the request carries the
 * sign-in `key` (or an unusable cookie). Sent as its own request right after
 * a sign-out, so it sees the browser's cookie at that moment, never the
 * cookie an older, possibly delayed request was issued with.
 */
export async function endSessionIfCurrent(key: unknown): Promise<boolean> {
  const session = await readSession()
  if (session.kind === "none") return true
  if (session.kind === "active" && (!isSignInKey(key) || signInKeyOf(session.credential) !== key)) return false
  await endSession()
  return true
}

/** Non-secret key of the current sign-in ("none" / "expired" without one), for fencing and keying client state. */
export async function sessionKey(): Promise<string> {
  const session = await readSession()
  return session.kind === "active" ? signInKeyOf(session.credential) : session.kind
}
