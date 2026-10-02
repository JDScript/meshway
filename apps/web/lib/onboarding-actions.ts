"use server"

import { ControlApiError, claimFirstOwner, listAccountPasswordResets, loginAccount, logoutAccount } from "@meshway/control-client"
import { zLoginRequest, zNetworkSnapshot } from "@meshway/control-client/schemas"
import { redirect } from "next/navigation"

import { passwordProblem, type OwnerClaimState } from "@/lib/account-model"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { formField } from "@/lib/sign-in"
import { EXPIRED_LOGIN_PATH, endSession, LOGIN_PATH, readSession } from "@/lib/session"

function claimFailure(error: unknown, secrets: string[], email: string): OwnerClaimState {
  if (error instanceof ControlApiError) {
    if (error.status === 409) return { status: "error", message: "该网络已经有所有者，或这个邮箱已有账户（已有账户请选择“已有账户”）。", email }
    if (error.status === 429) return { status: "error", message: "尝试次数过多，请稍后再试。", email }
    if (error.status === 400 || error.status === 422) return { status: "error", message: "邮箱或密码不符合要求。", email }
  }
  let message = describeError(error)
  for (const secret of secrets) if (secret) message = message.replaceAll(secret, "[已隐藏]")
  return { status: "error", message: `认领失败：${message}`, email }
}

/**
 * First Owner of a network that has none, proven by the administrator token
 * (bootstrap) session. Either a new account with a real email and a password
 * the person chooses here, or an existing account proven by its own email and
 * password. Nothing else changes: existing nodes stay unassigned until an
 * Owner claims them, and the administrator session is kept as is.
 */
export async function claimOwnerAction(_previous: OwnerClaimState, formData: FormData): Promise<OwnerClaimState> {
  const email = formField(formData, "email").trim().slice(0, 254)
  const session = await readSession()
  if (session.kind !== "active") {
    if (session.kind === "expired") await endSession()
    redirect(session.kind === "expired" ? EXPIRED_LOGIN_PATH : LOGIN_PATH)
  }
  if (session.credential.kind !== "bootstrap") {
    return { status: "error", message: "认领第一个所有者需要管理员令牌会话。", email }
  }
  const bootstrap = session.credential.token
  const networkId = zNetworkSnapshot.shape.network_id.safeParse(formField(formData, "network_id"))
  if (!networkId.success) return { status: "error", message: "网络无效，请刷新后重试。", email }
  const kind = formField(formData, "kind")
  const password = formField(formData, "password")
  if (!email || !password) return { status: "error", message: "请输入邮箱和密码。", email }

  if (kind === "new_account") {
    if (password !== formField(formData, "password_confirmation")) return { status: "error", message: "两次输入的密码不一致。", email }
    const problem = passwordProblem(password)
    if (problem) return { status: "error", message: problem, email }
    try {
      await claimFirstOwner(controllerUrl(), bootstrap, networkId.data, { kind: "new_account", email, password })
    } catch (error) {
      if (isCredentialRejected(error)) {
        await endSession()
        redirect(EXPIRED_LOGIN_PATH)
      }
      return claimFailure(error, [password, bootstrap], email)
    }
    return { status: "claimed", email, networkId: networkId.data, note: null }
  }

  if (kind !== "existing_account") return { status: "error", message: "请选择新建账户或已有账户。", email }
  // An existing account proves itself with its own sign-in; the session made
  // for the proof is revoked right after the claim.
  const login = zLoginRequest.safeParse({ email, password })
  if (!login.success) return { status: "error", message: "邮箱或密码不正确。", email }
  let proof: string
  try {
    proof = (await loginAccount(controllerUrl(), login.data)).session_token
  } catch (error) {
    if (error instanceof ControlApiError && [400, 401, 422].includes(error.status)) {
      return { status: "error", message: "邮箱或密码不正确。", email }
    }
    return claimFailure(error, [password], email)
  }
  let result: OwnerClaimState
  try {
    await claimFirstOwner(controllerUrl(), bootstrap, networkId.data, { kind: "existing_account", session_token: proof })
    result = { status: "claimed", email, networkId: networkId.data, note: null }
  } catch (error) {
    if (isCredentialRejected(error) && !(await bootstrapStillValid(bootstrap))) {
      await revokeProof(proof)
      await endSession()
      redirect(EXPIRED_LOGIN_PATH)
    }
    result = claimFailure(error, [password, proof, bootstrap], email)
  }
  if (!(await revokeProof(proof)) && result.status === "claimed") {
    result = { ...result, note: "用于证明账户的临时登录会话未能确认撤销，它会在到期时自动失效。" }
  }
  return result
}

async function revokeProof(proof: string): Promise<boolean> {
  try {
    await logoutAccount(controllerUrl(), proof)
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
