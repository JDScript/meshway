"use server"

import {
  ControlApiError,
  getConnections,
  getMeshAccess,
  getTags,
  previewFlow,
  saveMeshAccess,
  type FlowQuery,
  type MeshAccessPolicy,
} from "@meshway/control-client"

import { ACCESS_LIMITS, accessSaveFailure } from "@/lib/access-model"
import type { AccessReloadResult, AccessSaveResult, FlowPreviewResult } from "@/lib/action-state"
import {
  controllerUrl,
  describeError,
  isCredentialRejected,
} from "@/lib/controller"
import { rejectActionSession, requireActionAdminToken } from "@/lib/session"

// Arguments arrive from the browser and are untrusted: every action checks the
// session first, and the generated SDK validates the request body against the
// generated runtime schema before anything is sent to the controller.

const MAX_REVISION = 4_294_967_295

function oversized(value: unknown, limit: number = ACCESS_LIMITS.requestBytes): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).length > limit
  } catch {
    return true
  }
}

/**
 * `sourceJsonc` is the editor's JSONC text (comments included); the
 * controller stores it only if it parses to exactly `policy`.
 */
export async function saveMeshAccessAction(
  expectedRevision: number,
  policy: MeshAccessPolicy,
  sourceJsonc: string | null
): Promise<AccessSaveResult> {
  const token = await requireActionAdminToken()
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
    return { status: "error", message: "访问授权超过大小上限，未保存。" }
  }
  try {
    const snapshot = await saveMeshAccess(controllerUrl(), token, body)
    return { status: "saved", snapshot }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      const status = error instanceof ControlApiError ? error.status : null
      return accessSaveFailure(status, expectedRevision, describeError(error, token))
    }
  }
  return rejectActionSession()
}

/** Fresh snapshot for an explicit reload; callers decide to discard the draft. */
export async function reloadMeshAccessAction(): Promise<AccessReloadResult> {
  const token = await requireActionAdminToken()
  const baseUrl = controllerUrl()
  try {
    const [snapshot, connections, tags] = await Promise.all([
      getMeshAccess(baseUrl, token),
      getConnections(baseUrl, token),
      getTags(baseUrl, token),
    ])
    return { status: "loaded", snapshot, inventory: connections.nodes, tags }
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

/**
 * The controller's plan for one flow under the *saved* access and routing
 * policies: authorization, route and whether the runtime could execute it.
 * A plan, never a trace.
 */
export async function previewFlowAction(flow: FlowQuery): Promise<FlowPreviewResult> {
  const token = await requireActionAdminToken()
  const body = { flow }
  if (oversized(body, 16 * 1024)) {
    return { status: "error", message: "流量参数过大，未发送。" }
  }
  try {
    const response = await previewFlow(controllerUrl(), token, body)
    return { status: "previewed", response }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return { status: "error", message: `流量预览失败：${describeError(error, token)}` }
    }
  }
  return rejectActionSession()
}
