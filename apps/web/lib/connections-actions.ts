"use server"

import {
  ControlApiError,
  getConnectionRuntimes,
  getConnections,
  getManagedListeners,
  listNodes,
  saveConnections,
  type ConnectionPlan,
} from "@meshway/control-client"

import type {
  ConnectionsReloadResult,
  ConnectionsSaveResult,
  NodeStatusResult,
  PartRead,
  RuntimeReadResult,
} from "@/lib/action-state"
import { CONNECTION_LIMITS } from "@/lib/connections-model"
import {
  controllerUrl,
  describeError,
  isCredentialRejected,
} from "@/lib/controller"
import { rejectActionSession } from "@/lib/session"
import { requireNetworkActionContext } from "@/lib/network-context"

// Arguments arrive from the browser and are untrusted: every action checks the
// session first, and the generated SDK validates the request body against the
// generated runtime schema before anything is sent to the controller.

const MAX_REVISION = 4_294_967_295

function oversized(value: unknown): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).length > CONNECTION_LIMITS.planBytes
  } catch {
    return true
  }
}

export async function saveConnectionsAction(
  networkId: string,
  expectedRevision: number,
  configuration: ConnectionPlan
): Promise<ConnectionsSaveResult> {
  const { token } = await requireNetworkActionContext(networkId)
  if (
    !Number.isSafeInteger(expectedRevision) ||
    expectedRevision < 1 ||
    expectedRevision > MAX_REVISION
  ) {
    return { status: "error", message: "编辑基础版本无效，请重新读取。" }
  }
  const body = { expected_revision: expectedRevision, configuration }
  if (oversized(body)) {
    return { status: "error", message: "连接配置超过大小上限，未保存。" }
  }
  try {
    const snapshot = await saveConnections(controllerUrl(), token, networkId, body)
    return { status: "saved", snapshot, fetchedAt: Date.now() }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      if (error instanceof ControlApiError && error.status === 409) {
        return {
          status: "conflict",
          message: `Controller 的配置版本已不是 r${expectedRevision}：节点、公钥、路由或连接配置在你编辑期间被修改。未保存任何内容，草稿仍保留在编辑器中。`,
        }
      }
      if (error instanceof ControlApiError && error.status === 422) {
        return {
          status: "invalid",
          message:
            "Controller 拒绝保存：连接配置没有通过校验（节点公钥 / 承载能力、监听器、地址或配置档引用），或超出容量。已保存的配置保持不变。",
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
export async function reloadConnectionsAction(networkId: string): Promise<ConnectionsReloadResult> {
  const { token } = await requireNetworkActionContext(networkId)
  try {
    const snapshot = await getConnections(controllerUrl(), token, networkId)
    return { status: "loaded", snapshot, fetchedAt: Date.now() }
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

/** Read-only runtime observations for the bounded background refresh. */
export async function readConnectionRuntimesAction(networkId: string): Promise<RuntimeReadResult> {
  const { token } = await requireNetworkActionContext(networkId)
  try {
    const snapshot = await getConnectionRuntimes(controllerUrl(), token, networkId)
    return { status: "loaded", snapshot, fetchedAt: Date.now() }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return { status: "error", message: `运行状态读取失败：${describeError(error, token)}` }
    }
  }
  return rejectActionSession()
}

/**
 * Read-only status refresh for the node and topology pages: inventory
 * (contact, IP receipts), connection snapshot (plan, connection receipts) and
 * runtime reports in one request, so a single bounded poll covers all of them.
 */
export async function readNodeStatusAction(networkId: string): Promise<NodeStatusResult> {
  const { token } = await requireNetworkActionContext(networkId)
  const base = controllerUrl()
  const [nodes, connections, runtimes, managed] = await Promise.allSettled([
    listNodes(base, token, networkId),
    getConnections(base, token, networkId),
    getConnectionRuntimes(base, token, networkId),
    getManagedListeners(base, token, networkId),
  ])
  const fetchedAt = Date.now()
  const settled = [nodes, connections, runtimes, managed]
  if (settled.some((part) => part.status === "rejected" && isCredentialRejected(part.reason))) {
    return rejectActionSession()
  }
  const part = <T,>(result: PromiseSettledResult<T>, label: string): PartRead<T> =>
    result.status === "fulfilled"
      ? { ok: true, data: result.value }
      : { ok: false, error: `${label}读取失败：${describeError(result.reason, token)}` }
  return {
    status: "loaded",
    fetchedAt,
    nodes: part(nodes, "节点"),
    connections: part(connections, "连接配置"),
    runtimes: part(runtimes, "运行状态"),
    managed: part(managed, "托管监听"),
  }
}
