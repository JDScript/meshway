"use server"

import {
  ControlApiError,
  changeAccountPassword,
  getAccountSession,
  loginAccount,
  revokeAccountSessions,
} from "@meshway/control-client"
import { redirect } from "next/navigation"

import { passwordProblem } from "@/lib/account-model"
import type { SignInKey } from "@/lib/action-scope"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { endSignIn, LOGIN_PATH, rejectActionSession, requireRenderedSignIn, startUserSession } from "@/lib/session"
import { formField } from "@/lib/sign-in"

// "My account": actions of the signed-in user on their own account. Each is
// fenced to the sign-in the page was rendered for. Passwords go browser →
// Web server → controller only; nothing here echoes them back.

export type PasswordChangeState =
  | { status: "idle" }
  | { status: "error"; message: string }

async function requireUser(signIn: unknown) {
  const credential = await requireRenderedSignIn(signIn)
  if (credential.kind !== "user") redirect("/setup")
  return credential
}

/**
 * Change this account's password. The controller revokes every existing
 * session of the account with it (this one too), so the Web signs in again
 * with the new password and replaces the cookie.
 */
export async function changePasswordAction(signIn: SignInKey, returnTo: string, _previous: PasswordChangeState, formData: FormData): Promise<PasswordChangeState> {
  const credential = await requireUser(signIn)
  const current = formField(formData, "current_password")
  const next = formField(formData, "new_password")
  if (!current || !next) return { status: "error", message: "请输入当前密码和新密码。" }
  if (next !== formField(formData, "new_password_confirmation")) return { status: "error", message: "两次输入的新密码不一致。" }
  const problem = passwordProblem(next)
  if (problem) return { status: "error", message: problem }
  if (next === current) return { status: "error", message: "新密码与当前密码相同。" }

  let email: string
  try {
    email = (await getAccountSession(controllerUrl(), credential.token)).user.email
    await changeAccountPassword(controllerUrl(), credential.token, { current_password: current, new_password: next })
  } catch (error) {
    if (error instanceof ControlApiError && error.status === 401) {
      // Either the current password or the session; only a rejected session signs out.
      try {
        await getAccountSession(controllerUrl(), credential.token)
      } catch (check) {
        if (isCredentialRejected(check)) return rejectActionSession()
      }
      return { status: "error", message: "当前密码不正确。" }
    }
    if (error instanceof ControlApiError && error.status === 429) return { status: "error", message: "尝试次数过多，请稍后再试。" }
    if (error instanceof ControlApiError && (error.status === 400 || error.status === 422)) return { status: "error", message: "新密码不符合要求。" }
    let message = describeError(error, credential.token)
    for (const secret of [current, next]) message = message.replaceAll(secret, "[已隐藏]")
    return { status: "error", message: `修改失败：${message}` }
  }

  // The old session is gone now; sign in again with the new password.
  let session
  try {
    session = await loginAccount(controllerUrl(), { email, password: next })
  } catch {
    await endSignIn(signIn)
    redirect(`${LOGIN_PATH}?reason=password-changed`)
  }
  if (!(await startUserSession(session.session_token, session.expires_at))) redirect(`${LOGIN_PATH}?reason=password-changed`)
  redirect(`${safeConsolePath(returnTo)}?changed=password`)
}

/** Sign out everywhere: every session of this account (this browser's included) is revoked. */
export async function revokeAllSessionsAction(signIn: SignInKey): Promise<{ status: "error"; message: string }> {
  const credential = await requireUser(signIn)
  try {
    const { user } = await getAccountSession(controllerUrl(), credential.token)
    await revokeAccountSessions(controllerUrl(), credential.token, { user_id: user.user_id })
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    return { status: "error", message: `撤销失败：${describeError(error, credential.token)}` }
  }
  await endSignIn(signIn)
  redirect(`${LOGIN_PATH}?reason=sessions-revoked`)
}

/** Only this console's own account page; never an arbitrary redirect. */
function safeConsolePath(value: string): string {
  return /^\/[0-9a-f]{32}\/account$/.test(value) ? value : "/"
}
