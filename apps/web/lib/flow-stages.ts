import type { PreviewFlowResponse } from "@meshway/control-client"

import type { ChannelStatus, TopologyObservation } from "./topology-model.ts"

// What can and cannot be said about one hypothetical flow, stage by stage,
// each from its own source of truth: the controller's combined preview of
// the saved access and routing policies, and agent-reported neighbor probes
// for each planned hop. Nothing is inferred across stages, and none of them
// is a delivered packet.

export type StageStatus =
  /** Access: the first matching rule (or the default) allows the flow. */
  | "allowed"
  /** Access or routing rejects the flow. */
  | "rejected"
  /** The saved policy is invalid, so everything is denied. */
  | "invalid"
  /** Routing planned a path (logical intent). */
  | "planned"
  /** A fresh, authenticated neighbor probe exists. */
  | "observed"
  /** Reported but not proven, or not configured at all. */
  | "missing"
  /** The runtime could install this exact path. */
  | "eligible"
  /** The runtime would not execute this flow. */
  | "ineligible"
  /** No data, or not applicable. */
  | "unknown"

export type MatchedDirection = "forward" | "reverse"

export type HopStage = {
  from: string
  to: string
  /** Null: the saved connection plan configures no channel for this pair. */
  adjacency: ChannelStatus | null
  status: StageStatus
}

export type FlowStage =
  | { key: "access"; status: StageStatus; ruleId: string | null; direction: MatchedDirection | null; reason: string }
  | { key: "route"; status: StageStatus; ruleId: string | null; reason: string; path: string[] }
  | { key: "neighbors"; status: StageStatus; hops: HopStage[]; reason: string }
  | { key: "execution"; status: StageStatus; reason: string }

export const stageLabels: Record<FlowStage["key"], string> = {
  access: "访问授权",
  route: "路由规划",
  neighbors: "逐跳邻接",
  execution: "数据面执行条件",
}

const pairKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

/** The generated type is a pattern-checked string; compare, never cast. */
export function matchedDirection(value: string | null): MatchedDirection | null {
  return value === "forward" ? "forward" : value === "reverse" ? "reverse" : null
}

/**
 * `observation` is null when connections are unavailable (or not read on
 * this page); `liveKnown` is false when no runtime report has been read.
 */
export function flowStages(
  response: PreviewFlowResponse,
  observation: TopologyObservation | null,
  liveKnown: boolean
): FlowStage[] {
  const { authorization, routing } = response
  const access: FlowStage = {
    key: "access",
    status:
      authorization.action === "allow" ? "allowed" : authorization.action === "reject" ? "rejected" : "invalid",
    ruleId: authorization.rule_id,
    direction: matchedDirection(authorization.matched_direction),
    reason: authorization.reason,
  }

  const routed = routing.action === "route"
  const route: FlowStage = {
    key: "route",
    status: routing.action === "route" ? "planned" : routing.action === "reject" ? "rejected" : "invalid",
    ruleId: routing.rule_id,
    reason: routing.reason,
    path: routing.path,
  }

  const hops: HopStage[] = []
  for (let index = 0; index + 1 < routing.path.length; index += 1) {
    const from = routing.path[index]
    const to = routing.path[index + 1]
    if (from === undefined || to === undefined) continue
    const adjacency = observation?.adjacencies.find((candidate) => candidate.key === pairKey(from, to)) ?? null
    const status: StageStatus =
      !observation || !liveKnown
        ? "unknown"
        : adjacency && (adjacency.status === "confirmed" || adjacency.status === "one_sided")
          ? "observed"
          : "missing"
    hops.push({ from, to, adjacency: adjacency?.status ?? null, status })
  }
  const neighbors: FlowStage = {
    key: "neighbors",
    status:
      !routed || hops.length === 0 || hops.some((hop) => hop.status === "unknown")
        ? "unknown"
        : hops.every((hop) => hop.status === "observed")
          ? "observed"
          : "missing",
    hops,
    reason: !routed
      ? "路由没有规划路径，没有需要检查的跳。"
      : hops.length === 0
        ? "规划路径只有源节点（本地出网），没有 Mesh 跳。"
        : !observation
          ? "这里没有连接与运行报告，无法对照邻接。"
          : !liveKnown
            ? "尚未读取运行报告，邻接未知。"
            : "邻接来自 agent 自报的加密 nonce 探测，只证明两点间通道，不证明这条流量能通过。",
  }

  const execution: FlowStage = {
    key: "execution",
    status: response.execution_supported ? "eligible" : "ineligible",
    reason: response.execution_reason,
  }
  return [access, route, neighbors, execution]
}
