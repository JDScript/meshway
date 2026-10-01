import type {
  ConnectionListenerObservation,
  ConnectionTransport,
  ConnectionNeighborObservation,
  ConnectionNeighborState,
  ConnectionRuntimeObservation,
  ConnectionRuntimeReason,
  ConnectionRuntimeState,
  ConnectionRuntimesSnapshot,
} from "@meshway/control-client"

import type { CandidateDraft, ListenerDraft } from "./connections-model.ts"

// Presentation of the agent's self-reported connection runtime. A report is
// what an authenticated agent says about itself; the controller only checks
// its scope. Nothing here is a controller trace, an IP-flow grant or proof
// that routed / TUN traffic works.

/**
 * - `unreported`: the controller has no lease for this node at all.
 * - `awaiting`: a lease exists but no report for it yet (new process / epoch).
 * - `withdrawn`: revoked or withdrawn; any resources are history.
 * - `outdated`: the report describes a different configuration revision.
 * - `expired`: received too long ago (server flag or aged past the TTL here).
 * - `fresh`: current configuration, received within the TTL.
 */
export type RuntimeFreshness = "unreported" | "awaiting" | "withdrawn" | "outdated" | "expired" | "fresh"

export function runtimeFor(
  snapshot: ConnectionRuntimesSnapshot | null,
  nodeId: string
): ConnectionRuntimeObservation | undefined {
  return snapshot?.nodes.find((observation) => observation.node_id === nodeId)
}

/** Seconds since the controller received the report, at `now`; null when never reported. */
export function reportAge(observation: ConnectionRuntimeObservation, now: number): number | null {
  if (observation.reported_at === null) return null
  const received = Date.parse(observation.reported_at)
  if (Number.isNaN(received)) return null
  return Math.max(0, (now - received) / 1000)
}

/**
 * Freshness at `now`. The server's `current` / `stale` flags are honoured as
 * read, and the report keeps ageing in the browser after it was fetched.
 */
export function runtimeFreshness(
  observation: ConnectionRuntimeObservation | undefined,
  now: number
): RuntimeFreshness {
  if (!observation) return "unreported"
  if (observation.state === "withdrawn") return "withdrawn"
  if (observation.state === "awaiting_report" || observation.reported_at === null) return "awaiting"
  if (!observation.current) return "outdated"
  const age = reportAge(observation, now)
  if (observation.stale || age === null || age >= observation.stale_after_seconds) return "expired"
  return "fresh"
}

export const freshnessLabels: Record<RuntimeFreshness, string> = {
  unreported: "未报告",
  awaiting: "等待首次报告",
  withdrawn: "已撤回",
  outdated: "报告对应旧配置",
  expired: "报告已过期",
  fresh: "最近报告",
}

export const freshnessHints: Record<RuntimeFreshness, string> = {
  unreported: "Controller 没有这个节点的运行报告：未以托管连接模式运行，或从未报告。不代表已断开。",
  awaiting: "Agent 进程已登记，但尚未提交当前进程的报告。",
  withdrawn: "节点已撤销或撤回运行状态，以下不代表当前连接。",
  outdated: "报告基于另一个配置版本（例如重启或配置变更后），不能代表当前配置的连接情况。",
  expired: "超过有效期没有收到新报告，以下只是历史记录。",
  fresh: "节点 agent 自报，controller 只校验范围，不独立验证。",
}

export const runtimeStateLabels: Record<ConnectionRuntimeState, string> = {
  awaiting_report: "等待报告",
  disabled: "未启用托管连接",
  applying: "正在应用",
  applied: "连接配置已应用",
  error: "错误",
  withdrawn: "已撤回",
}

export const reasonLabels: Record<ConnectionRuntimeReason, string> = {
  none: "",
  disabled: "未启用",
  invalid_configuration: "连接配置无效",
  profile_unavailable: "本地配置档不可用",
  bind_failed: "本机监听失败",
  resolve_failed: "域名解析失败",
  transport_failed: "承载连接失败",
  authentication_failed: "节点认证失败",
  probe_failed: "邻居探测失败",
  lease_superseded: "已被新的 agent 进程接管",
  lease_expired: "运行租约已过期",
  shutting_down: "正在关闭",
  internal_error: "内部错误",
}

export const neighborStateLabels: Record<ConnectionNeighborState, string> = {
  authenticated: "已认证",
  waiting: "等待中",
  unreachable: "不可达",
}

/**
 * Applied means the node reconciled its saved plan (its listeners, if any,
 * are installed); never that every neighbor is up.
 */
export function runtimeSummary(observation: ConnectionRuntimeObservation): string {
  const reason = reasonLabels[observation.reason]
  const state = runtimeStateLabels[observation.state]
  return reason ? `${state}：${reason}` : state
}

export function listenerObservation(
  observation: ConnectionRuntimeObservation | undefined,
  listenerId: string
): ConnectionListenerObservation | undefined {
  return observation?.listeners.find((listener) => listener.listener_id === listenerId)
}

export type NeighborCounts = { authenticated: number; waiting: number; unreachable: number }

export function neighborCounts(observation: ConnectionRuntimeObservation): NeighborCounts {
  const counts: NeighborCounts = { authenticated: 0, waiting: 0, unreachable: 0 }
  for (const peer of observation.peers) counts[peer.state] += 1
  return counts
}

/** The saved adjacency one candidate describes, from either side. */
export type CandidateRelation = {
  id: string
  dialerNodeId: string
  listenerId: string
  /** Owner of the listener. */
  acceptorNodeId: string
  transport: ConnectionTransport
}

/**
 * Peers reported for one configured candidate, matched on the whole
 * relation (peer, initiator, carrier, listener), never the candidate ID
 * alone: an ID kept across a retarget must not inherit the old target's
 * observation. The dialer reports its exact candidate ID; the acceptor may
 * report null when it cannot tell configured candidates apart.
 */
export function candidateNeighbors(
  observation: ConnectionRuntimeObservation | undefined,
  candidate: CandidateRelation,
  selfNodeId: string
): ConnectionNeighborObservation[] {
  if (!observation) return []
  const id = candidate.id.trim()
  const outgoing = selfNodeId === candidate.dialerNodeId
  if (!outgoing && selfNodeId !== candidate.acceptorNodeId) return []
  return observation.peers.filter(
    (peer) =>
      peer.transport === candidate.transport &&
      peer.listener_id === candidate.listenerId &&
      peer.initiator_node_id === candidate.dialerNodeId &&
      peer.peer_node_id === (outgoing ? candidate.acceptorNodeId : candidate.dialerNodeId) &&
      (outgoing ? peer.candidate_id === id : peer.candidate_id === null || peer.candidate_id === id)
  )
}

/** Neighbor state with its sanitized reason, e.g. "不可达：节点认证失败". */
export function neighborLabel(peer: ConnectionNeighborObservation): string {
  const reason = reasonLabels[peer.reason]
  const state = neighborStateLabels[peer.state]
  return reason ? `${state}：${reason}` : state
}

export function formatRtt(peer: ConnectionNeighborObservation): string {
  if (peer.state !== "authenticated" || peer.probe_rtt_ms === null) return "未测"
  // Whole milliseconds: 0 is a real sub-millisecond measurement.
  return peer.probe_rtt_ms === 0 ? "<1 ms" : `${peer.probe_rtt_ms} ms`
}

/**
 * A fresh report may annotate an editor row only when it describes exactly
 * the saved configuration the editor is based on. `current` alone proves the
 * report matches the controller's latest revision, not this editor's base
 * and certainly not an unsaved draft.
 */
export function reportMatchesBase(
  observation: ConnectionRuntimeObservation | undefined,
  baseRevision: number,
  now: number
): observation is ConnectionRuntimeObservation {
  return runtimeFreshness(observation, now) === "fresh" && observation?.revision === baseRevision
}

/**
 * Row-level observations for the editor. Only items unchanged against the
 * saved base, observed by a fresh report of exactly that base revision, get
 * a live badge; edited or new items have nothing applied to show yet.
 */
export function listenerRowObservation(
  snapshot: ConnectionRuntimesSnapshot | null,
  now: number,
  baseRevision: number,
  listener: Pick<ListenerDraft, "id" | "nodeId">,
  unchanged: boolean
): ConnectionListenerObservation | undefined {
  if (!unchanged) return undefined
  const observation = runtimeFor(snapshot, listener.nodeId)
  if (!reportMatchesBase(observation, baseRevision, now)) return undefined
  return listenerObservation(observation, listener.id.trim())
}

export function candidateRowNeighbors(
  snapshot: ConnectionRuntimesSnapshot | null,
  now: number,
  baseRevision: number,
  candidate: Pick<CandidateDraft, "id" | "dialerNodeId" | "listenerId">,
  listener: Pick<ListenerDraft, "nodeId" | "transport"> | undefined,
  /** Candidate and its referenced listener both unchanged against the base. */
  unchanged: boolean
): ConnectionNeighborObservation[] {
  if (!unchanged || !listener) return []
  const relation: CandidateRelation = {
    id: candidate.id,
    dialerNodeId: candidate.dialerNodeId,
    listenerId: candidate.listenerId,
    acceptorNodeId: listener.nodeId,
    transport: listener.transport,
  }
  for (const side of [candidate.dialerNodeId, listener.nodeId]) {
    const observation = runtimeFor(snapshot, side)
    if (!reportMatchesBase(observation, baseRevision, now)) continue
    const peers = candidateNeighbors(observation, relation, side)
    if (peers.length > 0) return peers
  }
  return []
}
