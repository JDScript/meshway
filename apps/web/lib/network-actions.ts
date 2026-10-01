"use server"

import { ControlApiError, getNetwork, saveNetwork, type NetworkSnapshot, type SaveNetworkRequest } from "@meshway/control-client"

import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { rejectActionSession, requireActionAdminToken } from "@/lib/session"

export type NetworkSaveResult =
  | { status: "saved"; snapshot: NetworkSnapshot }
  | { status: "conflict" | "invalid" | "error"; message: string }

export type NetworkReadResult =
  | { status: "loaded"; snapshot: NetworkSnapshot }
  | { status: "error"; message: string }

/** Global CAS; only the generated SDK constructs and validates the HTTP request. */
export async function saveNetworkAction(request: SaveNetworkRequest): Promise<NetworkSaveResult> {
  const token = await requireActionAdminToken()
  if (!Number.isSafeInteger(request.expected_revision) || request.expected_revision < 1 || request.expected_revision > 4_294_967_295) {
    return { status: "error", message: "编辑基础版本无效，请重新读取。" }
  }
  try {
    return { status: "saved", snapshot: await saveNetwork(controllerUrl(), token, request) }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      const status = error instanceof ControlApiError ? error.status : null
      if (status === 409) {
        return { status: "conflict", message: `全局配置已不是 r${request.expected_revision}，可能有标签、路由或其他设置更新。当前名称与 DNS 草稿保留，未覆盖最新配置。` }
      }
      return { status: status === 422 ? "invalid" : "error", message: `保存失败：${describeError(error, token)}` }
    }
  }
  return rejectActionSession()
}

export async function readNetworkAction(): Promise<NetworkReadResult> {
  const token = await requireActionAdminToken()
  try {
    return { status: "loaded", snapshot: await getNetwork(controllerUrl(), token) }
  } catch (error) {
    if (!isCredentialRejected(error)) return { status: "error", message: `重新读取失败：${describeError(error, token)}` }
  }
  return rejectActionSession()
}
