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
import { rejectActionSession } from "@/lib/session"
import { requireNetworkActionContext } from "@/lib/network-context"

// Arguments arrive from the browser and are untrusted: every action checks the
// session first, and the generated SDK validates each request body against the
// generated runtime schema before anything is sent to the controller.

const MAX_REVISION = 4_294_967_295
// Policy cap plus the JSONC source (256 KiB) and room for the flow query;
// the controller enforces its own.
const MAX_REQUEST_BYTES = POLICY_LIMITS.policyBytes + 256 * 1024 + 16 * 1024

function oversized(value: unknown): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).length > MAX_REQUEST_BYTES
  } catch {
    return true
  }
}

export async function previewPolicyAction(
  networkId: string,
  policy: RoutingPolicy,
  flow: FlowQuery
): Promise<PreviewResult> {
  const { token } = await requireNetworkActionContext(networkId)
  const body = { policy, flow }
  if (oversized(body)) {
    return { status: "error", message: "策略超过大小上限，未发送。" }
  }
  try {
    const response = await previewPolicy(controllerUrl(), token, networkId, body)
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

/**
 * `sourceJsonc` is the editor's JSONC text (comments included); the
 * controller stores it only if it parses to exactly `policy`.
 */
export async function savePolicyAction(
  networkId: string,
  expectedRevision: number,
  policy: RoutingPolicy,
  sourceJsonc: string | null
): Promise<SaveResult> {
  const { token } = await requireNetworkActionContext(networkId)
  if (
    !Number.isSafeInteger(expectedRevision) ||
    expectedRevision < 1 ||
    expectedRevision > MAX_REVISION
  ) {
    return { status: "error", message: "编辑基础版本无效，请重新读取。" }
  }
  if (sourceJsonc !== null && typeof sourceJsonc !== "string") {
    return { status: "error", message: "JSONC 原文无效，未保存。" }
  }
  const body = { expected_revision: expectedRevision, policy, source_jsonc: sourceJsonc }
  if (oversized(body)) {
    return { status: "error", message: "策略超过大小上限，未保存。" }
  }
  try {
    const snapshot = await savePolicy(controllerUrl(), token, networkId, body)
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
          message: `Controller 拒绝保存：${describeError(error, token)}。已保存的策略保持不变；请运行「编译预览」查看诊断并修复。`,
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
export async function reloadPolicyAction(networkId: string): Promise<ReloadResult> {
  const { token } = await requireNetworkActionContext(networkId)
  const baseUrl = controllerUrl()
  try {
    const [snapshot, nodes] = await Promise.all([
      getPolicy(baseUrl, token, networkId),
      listNodes(baseUrl, token, networkId),
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
