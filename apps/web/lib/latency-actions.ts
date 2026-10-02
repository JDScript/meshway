"use server"

import { queryLatencyHistory } from "@meshway/control-client"
import { zQueryLatencyHistoryRequest } from "@meshway/control-client/schemas"

import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { PAGE_LIMIT, type LatencyPageResult } from "@/lib/latency-history-model"
import type { ActionScope } from "@/lib/action-scope"
import { requireNetworkActionContext } from "@/lib/network-context"
import { rejectActionSession } from "@/lib/session"

const MAX_WINDOW_MS = 24 * 60 * 60_000

/**
 * One read-only page of a directed pair's latency history. The query is a
 * POST to the controller but never writes; like every action it checks the
 * signed-in session and the network scope first, and the generated client
 * validates the body against the generated schema before sending it.
 */
export async function readLatencyHistoryPageAction(
  scope: ActionScope,
  input: { sourceNodeId: string; peerNodeId: string; from: string; until: string; after: string | null }
): Promise<LatencyPageResult> {
  const { token, networkId } = await requireNetworkActionContext(scope)
  const parsed = zQueryLatencyHistoryRequest.safeParse({
    source_node_id: input.sourceNodeId,
    peer_node_id: input.peerNodeId,
    from: input.from,
    until: input.until,
    after: input.after,
    limit: PAGE_LIMIT,
  })
  if (!parsed.success) return { status: "error", message: "查询参数无效。" }
  const span = Date.parse(parsed.data.until) - Date.parse(parsed.data.from)
  if (!(span > 0 && span <= MAX_WINDOW_MS)) return { status: "error", message: "时间范围必须在 24 小时以内。" }
  try {
    const page = await queryLatencyHistory(controllerUrl(), token, networkId, parsed.data)
    return { status: "loaded", page }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return { status: "error", message: `延迟历史读取失败：${describeError(error, token)}` }
    }
  }
  return rejectActionSession()
}
