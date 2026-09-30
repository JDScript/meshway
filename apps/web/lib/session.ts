import "server-only"

import { cookies, headers } from "next/headers"
import { redirect } from "next/navigation"

// Single-administrator console: the administrator token itself is kept in an
// HttpOnly, SameSite=Strict cookie and only ever read on the server. The
// cookie carries its own absolute expiry so the lifetime is enforced here and
// not only by the browser's Max-Age handling.
const COOKIE_NAME = "meshway_admin_session"
const COOKIE_VERSION = "v1"
const SESSION_LIFETIME_SECONDS = 8 * 60 * 60

export const LOGIN_PATH = "/login"
export const EXPIRED_LOGIN_PATH = "/login?reason=expired"

export type SessionState =
  | { kind: "none" }
  | { kind: "expired" }
  | { kind: "active"; token: string }

function parseSession(value: string): SessionState {
  const [version, expiresAt, ...rest] = value.split(".")
  const token = rest.join(".")
  const expiresAtSeconds = Number(expiresAt)
  if (version !== COOKIE_VERSION || !Number.isSafeInteger(expiresAtSeconds) || !token) {
    return { kind: "expired" }
  }
  if (expiresAtSeconds <= Math.floor(Date.now() / 1000)) return { kind: "expired" }
  return { kind: "active", token }
}

export async function readSession(): Promise<SessionState> {
  const cookie = (await cookies()).get(COOKIE_NAME)
  if (!cookie) return { kind: "none" }
  return parseSession(cookie.value)
}

/** For Server Component renders: redirects when no usable session exists. */
export async function requireAdminToken(): Promise<string> {
  const session = await readSession()
  if (session.kind === "active") return session.token
  redirect(session.kind === "expired" ? EXPIRED_LOGIN_PATH : LOGIN_PATH)
}

/** For Server Actions: additionally drops an expired cookie before redirecting. */
export async function requireActionAdminToken(): Promise<string> {
  const session = await readSession()
  if (session.kind === "active") return session.token
  if (session.kind === "expired") {
    await endSession()
    redirect(EXPIRED_LOGIN_PATH)
  }
  redirect(LOGIN_PATH)
}

/**
 * MESHWAY_WEB_COOKIE_SECURE: "true" always sets Secure, "false" never does,
 * unset or "auto" sets it when the browser reached the console over HTTPS.
 * Any other value fails closed to Secure.
 */
async function secureCookie(): Promise<boolean> {
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

async function writeCookie(value: string, maxAge: number) {
  const secure = await secureCookie()
  ;(await cookies()).set(COOKIE_NAME, value, {
    httpOnly: true,
    sameSite: "strict",
    secure,
    path: "/",
    maxAge,
    priority: "high",
  })
}

/** Server Actions only. */
export async function startSession(token: string) {
  const expiresAt = Math.floor(Date.now() / 1000) + SESSION_LIFETIME_SECONDS
  await writeCookie(
    `${COOKIE_VERSION}.${expiresAt}.${token}`,
    SESSION_LIFETIME_SECONDS
  )
}

/** Server Actions only. */
export async function endSession() {
  await writeCookie("", 0)
}
