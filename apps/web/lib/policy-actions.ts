"use server"

import {
  ControlApiError,
  getPolicy,
  listNodes,
  previewPolicy,
  savePolicy,
  type FlowQuery,
  type RoutingPolicy,
} from "@meshway/control-client"

import type { PreviewResult, ReloadResult, SaveResult } from "@/lib/action-state"
import {
  controllerUrl,
  describeError,
  isCredentialRejected,
} from "@/lib/controller"
import { POLICY_LIMITS } from "@/lib/policy-model"
import { rejectActionSession, requireActionAdminToken } from "@/lib/session"

// Arguments arrive from the browser and are untrusted: every action checks the
// session first, and the generated SDK validates each request body against the
// generated runtime schema before anything is sent to the controller.

const MAX_REVISION = 4_294_967_295
// Policy cap plus room for the flow query; the controller enforces its own.
const MAX_REQUEST_BYTES = POLICY_LIMITS.policyBytes + 16 * 1024

function oversized(value: unknown): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).length > MAX_REQUEST_BYTES
  } catch {
    return true
  }
}

export async function previewPolicyAction(
  policy: RoutingPolicy,
  flow: FlowQuery
): Promise<PreviewResult> {
  const token = await requireActionAdminToken()
  const body = { policy, flow }
  if (oversized(body)) {
    return { status: "error", message: "策略超过大小上限，未发送。" }
  }
  try {
    const response = await previewPolicy(controllerUrl(), token, body)
    return { status: "previewed", response }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return {
        status: "error",
        message: `编译预览失败：${describeError(error, token)}`,
      }
    }
  }
  return rejectActionSession()
}

export async function savePolicyAction(
  expectedRevision: number,
  policy: RoutingPolicy
): Promise<SaveResult> {
  const token = await requireActionAdminToken()
  if (
    !Number.isSafeInteger(expectedRevision) ||
    expectedRevision < 1 ||
    expectedRevision > MAX_REVISION
  ) {
    return { status: "error", message: "编辑基础版本无效，请重新读取。" }
  }
  const body = { expected_revision: expectedRevision, policy }
  if (oversized(body)) {
    return { status: "error", message: "策略超过大小上限，未保存。" }
  }
  try {
    const snapshot = await savePolicy(controllerUrl(), token, body)
    return { status: "saved", snapshot }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      if (error instanceof ControlApiError && error.status === 409) {
        return {
          status: "conflict",
          message: `Controller 的配置版本已不是 ${expectedRevision}：节点清单、国家或策略在你编辑期间被修改。未保存任何内容，草稿仍保留在编辑器中。`,
        }
      }
      if (error instanceof ControlApiError && error.status === 422) {
        return {
          status: "invalid",
          message: "Controller 拒绝保存：策略没有通过编译（或配置版本号已耗尽）。已保存的策略保持不变；请运行「编译预览」查看诊断并修复。",
        }
      }
      return {
        status: "error",
        message: `保存失败：${describeError(error, token)}`,
      }
    }
  }
  return rejectActionSession()
}

/** Fresh snapshot for an explicit reload; callers decide to discard the draft. */
export async function reloadPolicyAction(): Promise<ReloadResult> {
  const token = await requireActionAdminToken()
  const baseUrl = controllerUrl()
  try {
    const [snapshot, nodes] = await Promise.all([
      getPolicy(baseUrl, token),
      listNodes(baseUrl, token),
    ])
    return { status: "loaded", snapshot, nodes: nodes.nodes }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return {
        status: "error",
        message: `重新读取失败：${describeError(error, token)}`,
      }
    }
  }
  return rejectActionSession()
}
