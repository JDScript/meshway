"use server"

import {
  ControlApiError,
  claimFirstOwner,
  createAccountPasswordReset,
  listAccountPasswordResets,
  loginAccount,
  logoutAccount,
  revokeAccountPasswordReset,
  revokeAccountSessions,
} from "@meshway/control-client"
import { zCreatePasswordResetRequest, zLoginRequest, zNetworkSnapshot, zRevokePasswordResetRequest } from "@meshway/control-client/schemas"
import { refresh } from "next/cache"
import { redirect } from "next/navigation"

import { passwordProblem, type OwnerClaimState } from "@/lib/account-model"
import type { SignInKey } from "@/lib/action-scope"
import { SIGN_IN_FIELD } from "@/lib/action-scope"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { resetPath } from "@/lib/members-model"
import { consoleRoutes } from "@/lib/routes"
import { EXPIRED_LOGIN_PATH, requireRenderedSignIn, startUserSession } from "@/lib/session"
import { formField } from "@/lib/sign-in"

// Instance setup and operator recovery. Every action here needs the
// instance administrator token session (bootstrap) and is fenced to the
// sign-in its page was rendered for. A network Owner never reaches these:
// network roles are not instance authority.

async function requireBootstrap(signIn: unknown): Promise<string> {
  const credential = await requireRenderedSignIn(signIn)
  if (credential.kind !== "bootstrap") redirect("/setup")
  return credential.token
}

function redact(message: string, secrets: string[]): string {
  for (const secret of secrets) if (secret) message = message.replaceAll(secret, "[已隐藏]")
  return message
}

function claimFailure(error: unknown, secrets: string[], email: string): OwnerClaimState {
  if (error instanceof ControlApiError) {
    if (error.status === 409) return { status: "error", message: "该网络已经有所有者，或这个邮箱已有账户（已有账户请选择“已有账户”）。", email }
    if (error.status === 429) return { status: "error", message: "尝试次数过多，请稍后再试。", email }
    if (error.status === 400 || error.status === 422) return { status: "error", message: "邮箱或密码不符合要求。", email }
  }
  return { status: "error", message: `认领失败：${redact(describeError(error), secrets)}`, email }
}

async function revokeSession(token: string): Promise<boolean> {
  try {
    await logoutAccount(controllerUrl(), token)
    return true
  } catch (error) {
    return isCredentialRejected(error)
  }
}

async function bootstrapStillValid(token: string): Promise<boolean> {
  try {
    await listAccountPasswordResets(controllerUrl(), token)
    return true
  } catch (error) {
    return !isCredentialRejected(error)
  }
}

/**
 * First Owner of a network that has none, proven by the administrator token.
 * Either a new account (real email, password chosen by that person here) or
 * an existing account proven by its own email and password. With
 * `continue=on` the browser then signs in as that Owner and enters the
 * network: the administrator cookie is replaced, the token never becomes
 * part of the account. Existing nodes stay unassigned until an Owner claims them.
 */
export async function claimOwnerAction(_previous: OwnerClaimState, formData: FormData): Promise<OwnerClaimState> {
  const email = formField(formData, "email").trim().slice(0, 254)
  const bootstrap = await requireBootstrap(formData.get(SIGN_IN_FIELD))
  const networkId = zNetworkSnapshot.shape.network_id.safeParse(formField(formData, "network_id"))
  if (!networkId.success) return { status: "error", message: "网络无效，请刷新后重试。", email }
  const kind = formField(formData, "kind")
  const password = formField(formData, "password")
  const enter = formField(formData, "continue") === "on"
  if (!email || !password) return { status: "error", message: "请输入邮箱和密码。", email }

  // The user session that proves (existing) or results from (new) the claim.
  let session: { token: string; expiresAt: number }
  if (kind === "new_account") {
    if (password !== formField(formData, "password_confirmation")) return { status: "error", message: "两次输入的密码不一致。", email }
    const problem = passwordProblem(password)
    if (problem) return { status: "error", message: problem, email }
    try {
      await claimFirstOwner(controllerUrl(), bootstrap, networkId.data, { kind: "new_account", email, password })
    } catch (error) {
      if (isCredentialRejected(error)) redirect(EXPIRED_LOGIN_PATH)
      return claimFailure(error, [password, bootstrap], email)
    }
    if (!enter) return { status: "claimed", email, networkId: networkId.data, note: null }
    try {
      const login = await loginAccount(controllerUrl(), { email, password })
      session = { token: login.session_token, expiresAt: login.expires_at }
    } catch {
      return { status: "claimed", email, networkId: networkId.data, note: "所有者已认领，但自动登录失败；请退出后用该账户登录。" }
    }
  } else if (kind === "existing_account") {
    const login = zLoginRequest.safeParse({ email, password })
    if (!login.success) return { status: "error", message: "邮箱或密码不正确。", email }
    try {
      const proof = await loginAccount(controllerUrl(), login.data)
      session = { token: proof.session_token, expiresAt: proof.expires_at }
    } catch (error) {
      if (error instanceof ControlApiError && [400, 401, 422].includes(error.status)) {
        return { status: "error", message: "邮箱或密码不正确。", email }
      }
      return claimFailure(error, [password], email)
    }
    try {
      await claimFirstOwner(controllerUrl(), bootstrap, networkId.data, { kind: "existing_account", session_token: session.token })
    } catch (error) {
      const result = claimFailure(error, [password, session.token, bootstrap], email)
      const bootstrapRejected = isCredentialRejected(error) && !(await bootstrapStillValid(bootstrap))
      await revokeSession(session.token)
      if (bootstrapRejected) redirect(EXPIRED_LOGIN_PATH)
      return result
    }
    if (!enter) {
      const revoked = await revokeSession(session.token)
      return {
        status: "claimed",
        email,
        networkId: networkId.data,
        note: revoked ? null : "用于证明账户的临时登录会话未能确认撤销，它会在到期时自动失效。",
      }
    }
  } else {
    return { status: "error", message: "请选择新建账户或已有账户。", email }
  }

  if (!(await startUserSession(session.token, session.expiresAt))) {
    return { status: "claimed", email, networkId: networkId.data, note: "所有者已认领，但登录会话已过期；请检查 Web 与 controller 的系统时间后登录。" }
  }
  redirect(consoleRoutes(networkId.data).members)
}

export type ResetIssueState =
  | { status: "idle" }
  | { status: "error"; message: string }
  /** `path` carries the one-time secret in its fragment; shown once, never stored. */
  | { status: "created"; email: string; expiresAt: number; path: string }

const resetTtls = [15 * 60, 60 * 60]

/** A one-time password reset link for one account, handed over manually (no e-mail is sent). */
export async function createResetAction(signIn: SignInKey, email: string, _previous: ResetIssueState, formData: FormData): Promise<ResetIssueState> {
  const bootstrap = await requireBootstrap(signIn)
  const ttl = Number(formField(formData, "expires_in_seconds"))
  if (!resetTtls.includes(ttl)) return { status: "error", message: "请选择有效期。" }
  const body = zCreatePasswordResetRequest.safeParse({ user_id: formField(formData, "user_id"), expires_in_seconds: ttl })
  if (!body.success) return { status: "error", message: "账户无效，请刷新后重试。" }
  try {
    const created = await createAccountPasswordReset(controllerUrl(), bootstrap, body.data)
    refresh()
    return { status: "created", email, expiresAt: created.reset.expires_at, path: resetPath(created.reset_token) }
  } catch (error) {
    if (isCredentialRejected(error)) redirect(EXPIRED_LOGIN_PATH)
    return { status: "error", message: `创建失败：${describeError(error, bootstrap)}` }
  }
}

export async function revokeResetAction(signIn: SignInKey, resetId: string): Promise<{ error: string | null }> {
  const bootstrap = await requireBootstrap(signIn)
  const body = zRevokePasswordResetRequest.safeParse({ reset_id: resetId })
  if (!body.success) return { error: "重置链接无效。" }
  try {
    await revokeAccountPasswordReset(controllerUrl(), bootstrap, body.data)
  } catch (error) {
    if (isCredentialRejected(error)) redirect(EXPIRED_LOGIN_PATH)
    return { error: `撤销失败：${describeError(error, bootstrap)}` }
  }
  refresh()
  return { error: null }
}

/** Sign one account out everywhere (for a lost device or a leaked password). */
export async function revokeUserSessionsAction(signIn: SignInKey, userId: string): Promise<{ error: string | null }> {
  const bootstrap = await requireBootstrap(signIn)
  if (typeof userId !== "string" || userId.length === 0 || userId.length > 256) return { error: "账户无效。" }
  try {
    await revokeAccountSessions(controllerUrl(), bootstrap, { user_id: userId })
  } catch (error) {
    if (isCredentialRejected(error)) redirect(EXPIRED_LOGIN_PATH)
    return { error: `撤销失败：${describeError(error, bootstrap)}` }
  }
  return { error: null }
}
