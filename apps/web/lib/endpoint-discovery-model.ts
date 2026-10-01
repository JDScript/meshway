import type { NodeEndpointDiscovery, StunEndpointObservation } from "@meshway/control-client"

export const discoveryStateLabels: Record<NodeEndpointDiscovery["state"], string> = {
  awaiting_report: "等待报告",
  disabled: "未启用探测",
  probing: "探测中",
  observed: "未验证候选观测",
  withdrawn: "已撤回",
  expired: "已过期",
  error: "探测错误",
}

export const discoveryOutcomeLabels: Record<StunEndpointObservation["outcome"], string> = {
  mapped: "映射观测",
  timeout: "超时",
  invalid_response: "无效响应",
  io_error: "I/O 错误",
  stun_error: "STUN 错误",
}

/** A leased self-observation is never peer authentication or reachability. */
export function observationUsable(report: NodeEndpointDiscovery, observation: StunEndpointObservation, now: number): boolean {
  return report.state === "observed" && report.reported_at !== null && report.current && !report.stale &&
    observation.outcome === "mapped" && observation.mapped_endpoint !== null &&
    Date.parse(observation.observed_at) <= now && Date.parse(observation.expires_at) > now
}

export function discoveryLabel(report: NodeEndpointDiscovery | undefined, now: number): string {
  if (!report || report.reported_at === null) return "尚无报告"
  if (report.state !== "observed") return discoveryStateLabels[report.state]
  if (!report.current) return "配置 / 运行代际已变更"
  if (report.stale || !report.sockets.some((socket) => socket.observations.some((observation) => observationUsable(report, observation, now)))) return "观测已失效"
  return discoveryStateLabels.observed
}
