"use server"

import {
  ControlApiError,
  claimNodeOwnership,
  createAccountInvitation,
  leaveAccountNetwork,
  revokeAccountInvitation,
  saveAccountMember,
} from "@meshway/control-client"
import {
  zAccountRole,
  zClaimNodeOwnershipRequest,
  zRevokeInvitationRequest,
  zSaveMembershipRequest,
} from "@meshway/control-client/schemas"
import { refresh } from "next/cache"
import { redirect } from "next/navigation"

import { invitationPath, type InvitationIssueState } from "@/lib/account-model"
import type { ActionScope } from "@/lib/action-scope"
import { controllerUrl, describeError, isCredentialRejected, isPermissionDenied } from "@/lib/controller"
import { requireNetworkActionContext } from "@/lib/network-context"
import { rejectActionSession } from "@/lib/session"
import { formField } from "@/lib/sign-in"

// Member, invitation and device-ownership mutations of one network. Each
// first checks the rendered sign-in and the explicit network (the scope),
// then the controller re-checks the caller's live role in the same
// transaction as the write. Nothing here trusts the role shown on the page.

export type MemberMutationResult = { status: "saved" } | { status: "error"; message: string }

function failure(error: unknown, token: string, conflict: string, denied: string): MemberMutationResult {
  if (isPermissionDenied(error)) return { status: "error", message: denied }
  if (error instanceof ControlApiError) {
    if (error.status === 409) return { status: "error", message: conflict }
    if (error.status === 404) return { status: "error", message: "目标已不存在，请刷新后重试。" }
    if (error.status === 400 || error.status === 422) return { status: "error", message: "请求无效，请刷新后重试。" }
  }
  return { status: "error", message: describeError(error, token) }
}

/** Change a member's role, or remove them (their devices in this network are revoked with it). */
export async function saveMemberAction(scope: ActionScope, userId: string, change: string): Promise<MemberMutationResult> {
  const { token, networkId } = await requireNetworkActionContext(scope)
  const body = zSaveMembershipRequest.safeParse({ user_id: userId, role: change })
  if (!body.success) return { status: "error", message: "成员或角色无效。" }
  try {
    await saveAccountMember(controllerUrl(), token, networkId, body.data)
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    return failure(
      error,
      token,
      "网络必须保留至少一个所有者：先把另一位成员设为所有者，再修改或移除这位。",
      "你的角色不能做这项修改：管理员只能移除成员，角色调整需要所有者。"
    )
  }
  refresh()
  return { status: "saved" }
}

const ttlOptions = [86_400, 3 * 86_400, 7 * 86_400]

/** One invitation link for one email and role; the secret is shown once and never stored by the Web. */
export async function createInvitationAction(
  scope: ActionScope,
  _previous: InvitationIssueState,
  formData: FormData
): Promise<InvitationIssueState> {
  const { token, networkId } = await requireNetworkActionContext(scope)
  const role = zAccountRole.safeParse(formField(formData, "role"))
  if (!role.success) return { status: "error", message: "请选择角色。" }
  const email = formField(formData, "email").trim().slice(0, 254)
  if (!email) return { status: "error", message: "请输入受邀人的邮箱。" }
  const ttl = Number(formField(formData, "expires_in_seconds"))
  if (!ttlOptions.includes(ttl)) return { status: "error", message: "请选择有效期。" }
  let created
  try {
    created = await createAccountInvitation(controllerUrl(), token, networkId, { email, role: role.data, expires_in_seconds: ttl })
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    if (isPermissionDenied(error)) return { status: "error", message: "你的角色不能发出这个角色的邀请。" }
    if (error instanceof ControlApiError && (error.status === 400 || error.status === 422)) {
      return { status: "error", message: "邮箱或有效期不符合要求。" }
    }
    if (error instanceof ControlApiError && error.status === 409) {
      return { status: "error", message: "这个邮箱已经是本网络成员。" }
    }
    return { status: "error", message: `创建邀请失败：${describeError(error, token)}` }
  }
  refresh()
  let path: string
  try {
    path = invitationPath(created.invitation_token)
  } catch {
    // Committed, but the link cannot be built; never echo the secret.
    return { status: "error", message: "邀请已创建，但无法生成链接。请在邀请列表中撤销它后重新创建。" }
  }
  return {
    status: "created",
    email: created.invitation.email,
    role: created.invitation.role,
    expiresAt: created.invitation.expires_at,
    path,
  }
}

export async function revokeInvitationAction(scope: ActionScope, invitationId: string): Promise<MemberMutationResult> {
  const { token, networkId } = await requireNetworkActionContext(scope)
  const body = zRevokeInvitationRequest.safeParse({ invitation_id: invitationId })
  if (!body.success) return { status: "error", message: "邀请无效。" }
  try {
    await revokeAccountInvitation(controllerUrl(), token, networkId, body.data)
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    return failure(error, token, "邀请已被使用或撤销。", "你的角色不能撤销这个邀请。")
  }
  refresh()
  return { status: "saved" }
}

/**
 * An Owner assigns a legacy (unassigned, active) node to a live member of
 * this network. The node keeps its identity, address and tags.
 */
export async function claimNodeAction(scope: ActionScope, nodeId: string, membershipId: string): Promise<MemberMutationResult> {
  const { token, kind, networkId } = await requireNetworkActionContext(scope)
  if (kind !== "user") return { status: "error", message: "分配设备需要所有者账户登录；实例管理员令牌不能分配。" }
  const body = zClaimNodeOwnershipRequest.safeParse({ node_id: nodeId, membership_id: membershipId })
  if (!body.success) return { status: "error", message: "节点或成员无效。" }
  try {
    await claimNodeOwnership(controllerUrl(), token, networkId, body.data)
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    return failure(error, token, "这个节点已经有归属，或已被撤销。", "只有本网络的所有者可以分配旧节点，且目标必须是本网络的现有成员。")
  }
  refresh()
  return { status: "saved" }
}

/** Leave this network: the membership and every device it owns here are revoked together. */
export async function leaveNetworkAction(scope: ActionScope): Promise<MemberMutationResult> {
  const { token, kind, networkId } = await requireNetworkActionContext(scope)
  if (kind !== "user") return { status: "error", message: "实例管理员令牌不是网络成员。" }
  try {
    await leaveAccountNetwork(controllerUrl(), token, networkId)
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    return failure(error, token, "你是这个网络唯一的所有者：先把另一位成员设为所有者，再离开。", "无法离开这个网络。")
  }
  redirect("/")
}
