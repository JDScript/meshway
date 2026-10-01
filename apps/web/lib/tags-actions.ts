"use server"

import {
  ControlApiError,
  getConnections,
  getMeshAccess,
  getTags,
  saveTags,
  setNodeTags,
  type TagDefinition,
} from "@meshway/control-client"

import type { TagReloadResult, TagSaveResult } from "@/lib/action-state"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { rejectActionSession } from "@/lib/session"
import { requireNetworkActionContext } from "@/lib/network-context"

// Tag catalog and node tag bindings. Both mutations use the global
// configuration revision as their CAS base; the generated SDK validates every
// request body before it is sent.

const MAX_REVISION = 4_294_967_295
const MAX_REQUEST_BYTES = 64 * 1024

function badRevision(revision: number): boolean {
  return !Number.isSafeInteger(revision) || revision < 1 || revision > MAX_REVISION
}

function oversized(value: unknown): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).length > MAX_REQUEST_BYTES
  } catch {
    return true
  }
}

function failure(error: unknown, token: string, expectedRevision: number): TagSaveResult {
  const status = error instanceof ControlApiError ? error.status : null
  if (status === 409) {
    return {
      status: "conflict",
      message: `Controller 的配置版本已不是 r${expectedRevision}：期间有其他修改。未保存任何内容。`,
    }
  }
  if (status === 422) {
    return { status: "invalid", message: `Controller 拒绝保存：${describeError(error, token)}` }
  }
  return { status: "error", message: `保存失败：${describeError(error, token)}` }
}

export async function saveTagsAction(networkId: string, expectedRevision: number, tags: TagDefinition[]): Promise<TagSaveResult> {
  const { token } = await requireNetworkActionContext(networkId)
  if (badRevision(expectedRevision)) return { status: "error", message: "编辑基础版本无效，请重新读取。" }
  const body = { expected_revision: expectedRevision, tags }
  if (oversized(body)) return { status: "error", message: "标签目录超过大小上限，未保存。" }
  try {
    return { status: "saved", snapshot: await saveTags(controllerUrl(), token, networkId, body) }
  } catch (error) {
    if (!isCredentialRejected(error)) return failure(error, token, expectedRevision)
  }
  return rejectActionSession()
}

export async function setNodeTagsAction(
  networkId: string,
  expectedRevision: number,
  nodeId: string,
  tags: string[]
): Promise<TagSaveResult> {
  const { token } = await requireNetworkActionContext(networkId)
  if (badRevision(expectedRevision)) return { status: "error", message: "编辑基础版本无效，请重新读取。" }
  const body = { expected_revision: expectedRevision, node_id: nodeId, tags }
  if (oversized(body)) return { status: "error", message: "节点标签超过大小上限，未保存。" }
  try {
    return { status: "saved", snapshot: await setNodeTags(controllerUrl(), token, networkId, body) }
  } catch (error) {
    if (!isCredentialRejected(error)) return failure(error, token, expectedRevision)
  }
  return rejectActionSession()
}

/** Catalog, inventory and saved access rules (for "referenced by") read together. */
export async function reloadTagsAction(networkId: string): Promise<TagReloadResult> {
  const { token } = await requireNetworkActionContext(networkId)
  const baseUrl = controllerUrl()
  try {
    const [snapshot, connections, access] = await Promise.all([
      getTags(baseUrl, token, networkId),
      getConnections(baseUrl, token, networkId),
      getMeshAccess(baseUrl, token, networkId),
    ])
    return { status: "loaded", snapshot, inventory: connections.nodes, access }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return { status: "error", message: `重新读取失败：${describeError(error, token)}` }
    }
  }
  return rejectActionSession()
}
