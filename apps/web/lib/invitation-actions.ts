"use server"

import {
  ControlApiError,
  acceptAccountInvitation,
  createAccountInvitation,
  getAccountSession,
  loginAccount,
  registerInvitedAccount,
} from "@meshway/control-client"
import { zAccountRole, zAcceptInvitationRequest, zNetworkSnapshot } from "@meshway/control-client/schemas"
import { redirect } from "next/navigation"

import { invitationPath, passwordProblem, type InvitationIssueState, type InvitationState } from "@/lib/account-model"
import { controllerUrl, describeError, isCredentialRejected, isPermissionDenied } from "@/lib/controller"
import { consoleRoutes } from "@/lib/routes"
import { EXPIRED_LOGIN_PATH, endSession, LOGIN_PATH, readSession, requireActionCredential, startUserSession } from "@/lib/session"
import { formField, revokePreviousUserSession } from "@/lib/sign-in"

const invitationRejected = "邀请链接无效、已过期、已被撤销或已使用，或邮箱与邀请不符。请向网络所有者索取新的链接。"

function invitationToken(formData: FormData): string | null {
  const parsed = zAcceptInvitationRequest.safeParse({ invitation_token: formField(formData, "invitation_token") })
  return parsed.success ? parsed.data.invitation_token : null
}

/**
 * New account through an invitation (there is no other sign-up). The account,
 * its membership and the consumed invitation are saved together by the
 * controller; the Web then signs in with the same password.
 */
export async function registerInvitationAction(_previous: InvitationState, formData: FormData): Promise<InvitationState> {
  const token = invitationToken(formData)
  if (!token) return { status: "error", message: invitationRejected }
  const email = formField(formData, "email").trim()
  const password = formField(formData, "password")
  if (!email || !password) return { status: "error", message: "请输入邮箱和密码。" }
  if (password !== formField(formData, "password_confirmation")) return { status: "error", message: "两次输入的密码不一致。" }
  const problem = passwordProblem(password)
  if (problem) return { status: "error", message: problem }

  let networkId: string
  try {
    networkId = (await registerInvitedAccount(controllerUrl(), { email, password, invitation_token: token })).network_id
  } catch (error) {
    if (error instanceof ControlApiError) {
      if (error.status === 401) return { status: "error", message: invitationRejected }
      if (error.status === 409) return { status: "error", message: "这个邮箱已有账户。请先登录该账户，再重新打开邀请链接接受邀请。" }
      if (error.status === 429) return { status: "error", message: "尝试次数过多，请稍后再试。" }
      if (error.status === 400 || error.status === 422) return { status: "error", message: "邮箱或密码不符合要求。" }
    }
    return { status: "error", message: `注册失败：${describeError(error, password)}` }
  }
  let session
  try {
    session = await loginAccount(controllerUrl(), { email, password })
  } catch {
    // The account exists; only the automatic sign-in failed.
    redirect(LOGIN_PATH)
  }
  await revokePreviousUserSession()
  if (!(await startUserSession(session.session_token, session.expires_at))) redirect(LOGIN_PATH)
  redirect(consoleRoutes(networkId).overview)
}

/** An existing, signed-in account joins the invited network; its password is untouched. */
export async function acceptInvitationAction(_previous: InvitationState, formData: FormData): Promise<InvitationState> {
  const session = await readSession()
  if (session.kind !== "active" || session.credential.kind !== "user") {
    return { status: "error", message: "请先用账户登录，再重新打开邀请链接。" }
  }
  const token = invitationToken(formData)
  if (!token) return { status: "error", message: invitationRejected }
  let networkId: string
  try {
    networkId = (await acceptAccountInvitation(controllerUrl(), session.credential.token, { invitation_token: token })).network_id
  } catch (error) {
    if (error instanceof ControlApiError && error.status === 401) {
      // Either the invitation or the session; only a rejected session signs out.
      try {
        await getAccountSession(controllerUrl(), session.credential.token)
      } catch (check) {
        if (isCredentialRejected(check)) {
          await endSession()
          redirect(EXPIRED_LOGIN_PATH)
        }
      }
      return { status: "error", message: invitationRejected }
    }
    if (error instanceof ControlApiError && error.status === 409) return { status: "error", message: "你已经是该网络的成员。" }
    return { status: "error", message: `接受邀请失败：${describeError(error, session.credential.token)}` }
  }
  redirect(consoleRoutes(networkId).overview)
}

const ttlOptions = [86_400, 3 * 86_400, 7 * 86_400]

/** One invitation link for one email and role; the secret is shown once and never stored by the Web. */
export async function createInvitationAction(_previous: InvitationIssueState, formData: FormData): Promise<InvitationIssueState> {
  const credential = await requireActionCredential()
  const networkId = zNetworkSnapshot.shape.network_id.safeParse(formField(formData, "network_id"))
  if (!networkId.success) return { status: "error", message: "网络无效。" }
  const role = zAccountRole.safeParse(formField(formData, "role"))
  if (!role.success) return { status: "error", message: "请选择角色。" }
  const email = formField(formData, "email").trim()
  if (!email) return { status: "error", message: "请输入受邀人的邮箱。" }
  const ttl = Number(formField(formData, "expires_in_seconds"))
  if (!ttlOptions.includes(ttl)) return { status: "error", message: "请选择有效期。" }
  try {
    const created = await createAccountInvitation(controllerUrl(), credential.token, networkId.data, {
      email,
      role: role.data,
      expires_in_seconds: ttl,
    })
    return {
      status: "created",
      email: created.invitation.email,
      role: created.invitation.role,
      expiresAt: created.invitation.expires_at,
      path: invitationPath(created.invitation_token),
    }
  } catch (error) {
    if (isCredentialRejected(error)) {
      await endSession()
      redirect(EXPIRED_LOGIN_PATH)
    }
    if (isPermissionDenied(error)) return { status: "error", message: "你在该网络没有发出这个角色邀请的权限。" }
    if (error instanceof ControlApiError && (error.status === 400 || error.status === 422)) {
      return { status: "error", message: "邮箱或有效期不符合要求。" }
    }
    return { status: "error", message: `创建邀请失败：${describeError(error, credential.token)}` }
  }
}
