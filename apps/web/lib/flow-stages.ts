import type { PreviewPolicyResponse } from "@meshway/control-client"

import type { ChannelStatus, TopologyObservation } from "./topology-model.ts"

// What can and cannot be said about one hypothetical flow, stage by stage.
// Every stage comes from a real read: the controller's plan for the saved
// routing policy, and agent-reported neighbor probes for each planned hop.
// Stages without an API (access decision until the combined flow preview
// lands, data-plane execution, business traffic) are "unknown", never
// inferred from the others.

export type StageStatus =
  /** The controller planned it (logical intent). */
  | "planned"
  /** The controller rejected the flow, or the policy is invalid. */
  | "rejected"
  /** A fresh, authenticated neighbor probe exists. */
  | "observed"
  /** Reported, but not proven, or not configured at all. */
  | "missing"
  /** No API reports this. */
  | "unknown"

export type HopStage = {
  from: string
  to: string
  /** Null: the saved connection plan configures no channel for this pair. */
  adjacency: ChannelStatus | null
  status: StageStatus
}

export type FlowStage =
  | { key: "route"; status: StageStatus; ruleId: string | null; reason: string; path: string[] }
  | { key: "access"; status: "unknown"; reason: string }
  | { key: "neighbors"; status: StageStatus; hops: HopStage[]; reason: string }
  | { key: "dataplane"; status: "unknown"; reason: string }

export const stageLabels: Record<FlowStage["key"], string> = {
  route: "路由规划",
  access: "访问授权",
  neighbors: "逐跳邻接",
  dataplane: "数据面与业务流量",
}

const pairKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

/**
 * `observation` is null when connections are unavailable; `liveKnown` is
 * false when no runtime report has been read (adjacency unknown, not absent).
 */
export function flowStages(
  response: PreviewPolicyResponse,
  observation: TopologyObservation | null,
  liveKnown: boolean
): FlowStage[] {
  const { decision } = response
  const routed = decision.action === "route"
  const route: FlowStage = {
    key: "route",
    status: routed ? "planned" : "rejected",
    ruleId: decision.rule_id,
    reason: decision.reason,
    path: decision.path,
  }

  const hops: HopStage[] = []
  for (let index = 0; index + 1 < decision.path.length; index += 1) {
    const from = decision.path[index]
    const to = decision.path[index + 1]
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
    status: !routed
      ? "unknown"
      : hops.length === 0
        ? "unknown"
        : hops.some((hop) => hop.status === "unknown")
          ? "unknown"
          : hops.every((hop) => hop.status === "observed")
            ? "observed"
            : "missing",
    hops,
    reason: !routed
      ? "流量被拒绝，没有需要检查的跳。"
      : hops.length === 0
        ? "规划路径只有源节点（本地出网），没有 Mesh 跳。"
        : !observation
          ? "连接配置不可用，无法对照邻接。"
          : !liveKnown
            ? "尚未读取运行报告，邻接未知。"
            : "邻接来自 agent 自报的加密 nonce 探测，只证明两点间通道，不证明这条流量能通过。",
  }
  return [
    route,
    {
      key: "access",
      status: "unknown",
      reason: "当前接口不返回访问授权判定；controller 组合流量预览上线后在这里显示命中的访问规则与方向。",
    },
    neighbors,
    {
      key: "dataplane",
      status: "unknown",
      reason: "没有 TUN 安装或业务流量观测接口；请用 ping / curl 在节点上实测。",
    },
  ]
}
