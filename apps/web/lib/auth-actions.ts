"use server"

import { ControlApiError, loginAccount, logoutAccount } from "@meshway/control-client"
import { zLoginRequest } from "@meshway/control-client/schemas"
import { redirect } from "next/navigation"

import type { AccountLoginState } from "@/lib/account-model"
import type { LoginState } from "@/lib/action-state"
import { verifyCredential } from "@/lib/account-session"
import { safeLoginReturn } from "@/lib/device-login"
import { controllerUrl, describeError, isCredentialRejected, isPermissionDenied } from "@/lib/controller"
import type { SignInKey } from "@/lib/action-scope"
import { endSession, endSessionIfCurrent, LOGIN_PATH, readSession, requireRenderedSignIn, sessionKey, startBootstrapSession, startUserSession } from "@/lib/session"
import { isUserSessionToken } from "@/lib/session-cookie"
import { formField, revokePreviousUserSession } from "@/lib/sign-in"

const MAX_TOKEN_LENGTH = 1024
const VISIBLE_ASCII = /^[\x21-\x7E]+$/

/** Compatibility entry: the instance administrator token (bootstrap). */
export async function login(_previous: LoginState, formData: FormData): Promise<LoginState> {
  const token = formField(formData, "token").trim()
  if (!token) return { status: "malformed", message: "请输入管理员令牌。" }
  if (token.length > MAX_TOKEN_LENGTH || !VISIBLE_ASCII.test(token)) {
    return {
      status: "malformed",
      message: "管理员令牌只包含可见 ASCII 字符，不含空格。请检查是否完整复制。",
    }
  }
  // A user session is never accepted as the administrator credential.
  if (isUserSessionToken(token) || token.startsWith("mws_")) return { status: "invalid" }

  try {
    await verifyCredential({ kind: "bootstrap", token })
  } catch (error) {
    if (isCredentialRejected(error) || isPermissionDenied(error)) return { status: "invalid" }
    return { status: "unavailable", detail: describeError(error, token) }
  }

  await revokePreviousUserSession()
  await startBootstrapSession(token)
  redirect(safeLoginReturn(formData.get("returnTo")))
}

/**
 * Email + password sign-in. The password goes browser → Web server →
 * controller only; it is never stored, logged or returned in the state.
 * The resulting `mws_` session is kept in the HttpOnly cookie for exactly
 * the controller's absolute lifetime.
 */
export async function loginWithPassword(_previous: AccountLoginState, formData: FormData): Promise<AccountLoginState> {
  const email = formField(formData, "email").trim().slice(0, 254)
  const parsed = zLoginRequest.safeParse({ email, password: formField(formData, "password") })
  if (!parsed.success || !parsed.data.password) return { status: "malformed", message: "请输入邮箱和密码。", email }
  const { password } = parsed.data

  let response
  try {
    response = await loginAccount(controllerUrl(), parsed.data)
  } catch (error) {
    if (error instanceof ControlApiError) {
      // 400/401/422 all mean "these credentials do not sign in"; which part
      // was wrong is deliberately not distinguished.
      if ([400, 401, 422].includes(error.status)) return { status: "invalid", email }
      if (error.status === 429) return { status: "limited", email }
    }
    return { status: "unavailable", detail: describeError(error, password), email }
  }

  await revokePreviousUserSession()
  if (!(await startUserSession(response.session_token, response.expires_at))) {
    return { status: "unavailable", detail: "Controller 返回的会话已过期，请检查 Web 与 controller 的系统时间。", email }
  }
  redirect(safeLoginReturn(formData.get("returnTo")))
}

/**
 * Sign-out, step one: for the sign-in the page was rendered for, revoke a
 * user session on the controller. The cookie is not touched here, because
 * this response may arrive late, after another tab signed in; see
 * `discardSignInAction`. If the controller could not confirm the revocation,
 * the login page says so instead of claiming it happened.
 */
export async function signOutAction(signIn: SignInKey, returnTo: unknown): Promise<{ path: string }> {
  const credential = await requireRenderedSignIn(signIn)
  let outcome: "revoked" | "unconfirmed" | "local" = "local"
  if (credential.kind === "user") {
    try {
      await logoutAccount(controllerUrl(), credential.token)
      outcome = "revoked"
    } catch (error) {
      // 401: the controller no longer accepts it, which is the goal.
      outcome = isCredentialRejected(error) ? "revoked" : "unconfirmed"
    }
  }
  const destination = safeLoginReturn(returnTo)
  const query = new URLSearchParams()
  if (destination !== "/") query.set("returnTo", destination)
  if (outcome === "unconfirmed") query.set("reason", "logout-unconfirmed")
  const search = query.toString()
  return { path: search ? `${LOGIN_PATH}?${search}` : LOGIN_PATH }
}

/**
 * Sign-out, step two, sent as a fresh request: clears the cookie only if the
 * browser still holds that same sign-in. A newer sign-in is left alone.
 */
export async function discardSignInAction(signIn: SignInKey): Promise<void> {
  await endSessionIfCurrent(signIn)
}

/**
 * Drops the session cookie only when it is actually unusable: expired locally
 * or rejected by the controller. A still-valid session is left untouched, so
 * this cannot be abused to sign someone out, and a controller outage is not
 * mistaken for an authentication failure.
 */
export async function discardRejectedSession(): Promise<"none" | "cleared" | "kept" | "unavailable"> {
  // Runs as its own request from the login page, so this is the cookie the
  // browser holds now (not one an older request was issued with).
  const session = await readSession()
  if (session.kind === "none") return "none"
  if (session.kind === "expired") {
    await endSession()
    return "cleared"
  }
  try {
    await verifyCredential(session.credential)
  } catch (error) {
    if (isCredentialRejected(error)) {
      await endSession()
      return "cleared"
    }
    return "unavailable"
  }
  return "kept"
}

/** Non-secret key of the current sign-in; lets an open console notice an account switch. */
export async function currentSessionKey(): Promise<string> {
  return sessionKey()
}
