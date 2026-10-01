"use server"

import {
  ControlApiError,
  getConnections,
  getMeshAccess,
  saveMeshAccess,
  type MeshAccessPolicy,
} from "@meshway/control-client"

import { ACCESS_LIMITS, accessSaveFailure } from "@/lib/access-model"
import type { AccessReloadResult, AccessSaveResult } from "@/lib/action-state"
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

function oversized(value: unknown): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).length > ACCESS_LIMITS.requestBytes
  } catch {
    return true
  }
}

export async function saveMeshAccessAction(
  expectedRevision: number,
  policy: MeshAccessPolicy
): Promise<AccessSaveResult> {
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
    const [snapshot, connections] = await Promise.all([
      getMeshAccess(baseUrl, token),
      getConnections(baseUrl, token),
    ])
    return { status: "loaded", snapshot, inventory: connections.nodes }
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
