"use server"

import { ControlApiError, createNetwork, listNetworks, getNetwork, saveNetwork, type NetworkSnapshot, type SaveNetworkRequest } from "@meshway/control-client"
import { cookies } from "next/headers"
import { refresh } from "next/cache"
import { NETWORK_COOKIE, requireNetworkActionContext } from "@/lib/network-context"

import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { rejectActionSession, requireActionAdminToken, secureCookie } from "@/lib/session"

export type NetworkSaveResult =
  | { status: "saved"; snapshot: NetworkSnapshot }
  | { status: "conflict" | "invalid" | "error"; message: string }

export type NetworkReadResult =
  | { status: "loaded"; snapshot: NetworkSnapshot }
  | { status: "error"; message: string }

/** Global CAS; only the generated SDK constructs and validates the HTTP request. */
export async function saveNetworkAction(networkId: string, request: SaveNetworkRequest): Promise<NetworkSaveResult> {
  const { token } = await requireNetworkActionContext(networkId)
  if (!Number.isSafeInteger(request.expected_revision) || request.expected_revision < 1 || request.expected_revision > 4_294_967_295) {
    return { status: "error", message: "编辑基础版本无效，请重新读取。" }
  }
  try {
    return { status: "saved", snapshot: await saveNetwork(controllerUrl(), token, networkId, request) }
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

export async function readNetworkAction(networkId: string): Promise<NetworkReadResult> {
  const { token } = await requireNetworkActionContext(networkId)
  try {
    return { status: "loaded", snapshot: await getNetwork(controllerUrl(), token, networkId) }
  } catch (error) {
    if (!isCredentialRejected(error)) return { status: "error", message: `重新读取失败：${describeError(error, token)}` }
  }
  return rejectActionSession()
}

async function selectNetworkCookie(id: string) {
  ;(await cookies()).set(NETWORK_COOKIE, id, { httpOnly: true, sameSite: "strict", secure: await secureCookie(), path: "/", maxAge: 8 * 60 * 60 })
  refresh()
}

export async function switchNetworkAction(id: string): Promise<{ error: string | null }> {
  const token = await requireActionAdminToken()
  try {
    const networks = await listNetworks(controllerUrl(), token)
    if (!networks.networks.some(network => network.network_id === id)) return { error: "网络不存在。" }
    await selectNetworkCookie(id)
    return { error: null }
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    return { error: `切换失败：${describeError(error, token)}` }
  }
}

export async function createNetworkAction(_previous: { error: string | null }, form: FormData): Promise<{ error: string | null }> {
  const token = await requireActionAdminToken()
  const name = form.get("name")
  if (typeof name !== "string") return { error: "请输入网络名称。" }
  try {
    const network = await createNetwork(controllerUrl(), token, { name })
    await selectNetworkCookie(network.network_id)
    return { error: null }
  } catch (error) {
    if (isCredentialRejected(error)) return rejectActionSession()
    return { error: `创建失败：${describeError(error, token)}` }
  }
}
