"use server"

import {
  ControlApiError,
  createManagedEnrollmentToken,
  getConnections,
  getManagedListeners,
  saveManagedListener,
  type ConnectionsSnapshot,
} from "@meshway/control-client"
import { zManagedListenerSettings, zSaveManagedListenerRequest } from "@meshway/control-client/schemas"

import {
  enrollmentTtlOptions,
  type EnrollmentState,
  type ManagedListenerReadResult,
  type ManagedListenerSaveResult,
  type PartRead,
} from "@/lib/action-state"
import { describeError, isCredentialRejected } from "@/lib/controller"
import { requireNetworkActionContext } from "@/lib/network-context"
import { rejectActionSession } from "@/lib/session"

// Arguments arrive from the browser and are untrusted: each action checks the
// session and the rendered network first, then validates the body with the
// generated runtime schema. Responses never contain UUIDs, PEM or keys: the
// administrator API does not return them.

async function readConnections(base: string, token: string, networkId: string): Promise<PartRead<ConnectionsSnapshot>> {
  try {
    return { ok: true, data: await getConnections(base, token, networkId) }
  } catch (error) {
    if (isCredentialRejected(error)) throw error
    return { ok: false, error: `连接配置读取失败：${describeError(error, token)}` }
  }
}

export async function saveManagedListenerAction(
  networkId: string,
  expectedRevision: number,
  settings: unknown
): Promise<ManagedListenerSaveResult> {
  const { token, baseUrl } = await requireNetworkActionContext(networkId)
  const body = zSaveManagedListenerRequest.safeParse({ expected_revision: expectedRevision, settings })
  if (!body.success || body.data.settings.node_id === "") {
    return { status: "invalid", message: "表单内容不符合监听协议，未发送。" }
  }
  try {
    const managed = await saveManagedListener(baseUrl, token, networkId, body.data)
    const connections = await readConnections(baseUrl, token, networkId)
    return { status: "saved", managed, connections, fetchedAt: Date.now() }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      if (error instanceof ControlApiError && error.status === 409) {
        return {
          status: "conflict",
          message: `网络配置已不是 r${expectedRevision}，或这个监听 ID 已被其他节点 / 承载占用。未保存，表单保留。`,
        }
      }
      if (error instanceof ControlApiError && error.status === 422) {
        return {
          status: "invalid",
          message: "Controller 拒绝保存：监听地址、对外地址、TLS / WebSocket 组合无效，或超出容量。已保存的配置不变。",
        }
      }
      if (error instanceof ControlApiError && error.status === 404) {
        return { status: "not_found", message: "节点已撤销或不在这个网络中，未保存。" }
      }
      return { status: "error", message: `保存失败：${describeError(error, token)}` }
    }
  }
  return rejectActionSession()
}

export async function readManagedListenersAction(networkId: string): Promise<ManagedListenerReadResult> {
  const { token, baseUrl } = await requireNetworkActionContext(networkId)
  try {
    const managed = await getManagedListeners(baseUrl, token, networkId)
    const connections = await readConnections(baseUrl, token, networkId)
    return { status: "loaded", managed, connections, fetchedAt: Date.now() }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return { status: "error", message: `托管监听读取失败：${describeError(error, token)}`, fetchedAt: Date.now() }
    }
  }
  return rejectActionSession()
}

/**
 * One-time token whose enrollment also stores this listener intent for the
 * new node. The token value appears only in this response.
 */
export async function createManagedEnrollmentAction(
  networkId: string,
  expiresInSeconds: number,
  listener: unknown
): Promise<EnrollmentState> {
  const { token, baseUrl } = await requireNetworkActionContext(networkId)
  const option = enrollmentTtlOptions.find(({ seconds }) => seconds === expiresInSeconds)
  if (!option) return { status: "error", message: "请选择有效期。" }
  const initial = zManagedListenerSettings.safeParse(listener)
  if (!initial.success || initial.data.node_id !== "") {
    return { status: "error", message: "监听设置不符合协议，未生成令牌。" }
  }
  try {
    const enrollment = await createManagedEnrollmentToken(baseUrl, token, networkId, {
      expires_in_seconds: option.seconds,
      initial_listener: initial.data,
    })
    return { status: "created", enrollment, controllerUrl: baseUrl, createdAt: Date.now(), initialListener: initial.data }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      if (error instanceof ControlApiError && error.status === 422) {
        return { status: "error", message: "Controller 拒绝了监听设置（地址、TLS / WebSocket 组合无效）。未生成令牌，设置保留。" }
      }
      return { status: "error", message: `创建注册令牌失败：${describeError(error, token)}` }
    }
  }
  return rejectActionSession()
}
