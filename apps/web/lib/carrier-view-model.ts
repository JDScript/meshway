import type {
  ConnectionListenerObservation,
  ConnectionNeighborObservation,
  ConnectionPlan,
  ConnectionRuntimeObservation,
  ConnectionTransport,
  NodeEndpointDiscovery,
  StunEndpointObservation,
  UdpDiscoveryResourceKind,
  UdpSocketDiscovery,
} from "@meshway/control-client"

import { candidateNeighbors, reportMatchesBase } from "./connection-runtime-model.ts"
import { observationUsable } from "./endpoint-discovery-model.ts"

// One node's carriers as the console shows them: what is configured, what
// the agent says it bound, which external addresses exist and where each
// came from, and what peers verified. These are separate facts. A STUN
// mapping on a UDP socket is an unverified candidate for that socket only;
// it never makes a listener reachable, and never says anything about TCP
// or VLESS carriers.

export type ObservedState = "candidate" | "expired" | "historical" | "failed"

export type ObservedAddress = {
  /** Mapped IP:port; null when the observer returned no mapping. */
  address: string | null
  observer: string
  outcome: StunEndpointObservation["outcome"]
  state: ObservedState
  observedAt: string
  expiresAt: string
  rttMs: number | null
}

export type SocketFacts = {
  /**
   * The socket belongs to a discovery report that passed the local fence
   * (same node, same plan revision, server-current, and the same epoch /
   * digest as a fresh runtime report of that revision). Otherwise its bound
   * address and observations are history only. Independent of each
   * measurement's outcome: a STUN timeout does not mean the socket closed.
   */
  current: boolean
  boundAddress: string
  generation: number
  socketId: number
  source: UdpSocketDiscovery["source"]
  observed: ObservedAddress[]
}

/**
 * UDP address observation for one resource:
 * - `not_applicable`: not a UDP carrier; STUN says nothing about it.
 * - `none`: no socket of this resource in the discovery report.
 */
export type UdpFacts = { kind: "not_applicable" } | { kind: "none" } | { kind: "sockets"; sockets: SocketFacts[] }

export type PeerCheck = {
  /** Saved candidate that describes this adjacency. */
  candidateId: string
  peerNodeId: string
  /** Peer endpoint the dialer was told to use, host:port. */
  endpoint: string
  /** Matching neighbor reports from this node's fresh runtime report of the base revision. */
  reports: ConnectionNeighborObservation[]
}

export type ListenerView = {
  id: string
  transport: ConnectionTransport
  /** Saved bind address (configuration). */
  bind: string
  /** Administrator-published addresses for peers, host:port. */
  advertised: string[]
  /** Agent's own report of this listener; undefined when not usable for this revision. */
  runtime: ConnectionListenerObservation | undefined
  udp: UdpFacts
  /** Other nodes' saved candidates that dial into this listener. */
  incoming: PeerCheck[]
}

export type OutgoingView = {
  id: string
  transport: ConnectionTransport | null
  /** Listener owner being dialled; null when the listener is missing from the plan. */
  targetNodeId: string | null
  listenerId: string
  endpoint: string
  priority: number
  udp: UdpFacts
  check: PeerCheck
}

export type OrphanSocket = { kind: UdpDiscoveryResourceKind; resourceId: string; facts: SocketFacts }

export type CarrierView = {
  /** The node's discovery report passed the local fence; see `discoveryIsCurrent`. */
  discoveryCurrent: boolean
  listeners: ListenerView[]
  outgoing: OutgoingView[]
  /** Reported sockets whose resource is not in the saved plan (e.g. removed since). */
  orphans: OrphanSocket[]
}

const hostPort = (endpoint: { host: string; port: number }) =>
  endpoint.host.includes(":") && !endpoint.host.startsWith("[") ? `[${endpoint.host}]:${endpoint.port}` : `${endpoint.host}:${endpoint.port}`

/**
 * Whether a discovery report may stand as current evidence for `plan` at
 * `revision`. Discovery and runtime are read independently, so the report
 * must also share the epoch and digest of a fresh runtime report of the same
 * revision; with no such runtime the report is history.
 */
export function discoveryIsCurrent(
  report: NodeEndpointDiscovery,
  nodeId: string,
  revision: number,
  runtime: ConnectionRuntimeObservation | undefined
): boolean {
  return report.node_id === nodeId && report.state === "observed" && report.reported_at !== null &&
    report.current && !report.stale && report.revision === revision &&
    runtime !== undefined && runtime.node_id === nodeId && runtime.revision === revision &&
    runtime.epoch === report.epoch && runtime.digest !== null && runtime.digest === report.digest
}

export function observedState(report: NodeEndpointDiscovery, observation: StunEndpointObservation, now: number, current: boolean): ObservedState {
  if (current && observationUsable(report, observation, now)) return "candidate"
  if (observation.outcome !== "mapped" || observation.mapped_endpoint === null) return "failed"
  if (Date.parse(observation.expires_at) <= now) return "expired"
  return "historical"
}

export const observedStateLabels: Record<ObservedState, string> = {
  candidate: "未验证候选",
  expired: "已过期",
  historical: "历史观测",
  failed: "无映射",
}

/** Probe evidence for one neighbor: an authenticated channel without a fresh probe has no current RTT. */
export function peerProbe(peer: ConnectionNeighborObservation): { fresh: boolean; rtt: string | null } {
  if (peer.state !== "authenticated" || !peer.probe_fresh) return { fresh: false, rtt: null }
  if (peer.probe_rtt_ms === null) return { fresh: true, rtt: null }
  return { fresh: true, rtt: peer.probe_rtt_ms === 0 ? "<1 ms" : `${peer.probe_rtt_ms} ms` }
}

function socketFacts(report: NodeEndpointDiscovery, socket: UdpSocketDiscovery, now: number, current: boolean): SocketFacts {
  return {
    current,
    boundAddress: socket.bound_address,
    generation: socket.generation,
    socketId: socket.socket_id,
    source: socket.source,
    observed: socket.observations.map((observation) => ({
      address: observation.mapped_endpoint,
      observer: observation.observer,
      outcome: observation.outcome,
      state: observedState(report, observation, now, current),
      observedAt: observation.observed_at,
      expiresAt: observation.expires_at,
      rttMs: observation.rtt_ms,
    })),
  }
}

function udpFacts(
  transport: ConnectionTransport | null,
  kind: UdpDiscoveryResourceKind,
  id: string,
  report: NodeEndpointDiscovery | undefined,
  now: number,
  current: boolean
): UdpFacts {
  if (transport !== "native_udp") return { kind: "not_applicable" }
  const sockets = report?.sockets.filter((socket) => socket.resource_kind === kind && socket.resource_id === id) ?? []
  if (!report || sockets.length === 0) return { kind: "none" }
  return { kind: "sockets", sockets: sockets.map((socket) => socketFacts(report, socket, now, current)) }
}

export function nodeCarrierView({
  plan,
  revision,
  nodeId,
  runtime,
  discovery,
  now,
}: {
  plan: ConnectionPlan
  /** Revision of `plan`; runtime facts only annotate a report of exactly this revision. */
  revision: number
  nodeId: string
  runtime: ConnectionRuntimeObservation | undefined
  discovery: NodeEndpointDiscovery | undefined
  now: number
}): CarrierView {
  const usableRuntime = runtime?.node_id === nodeId && reportMatchesBase(runtime, revision, now) ? runtime : undefined
  // Another node's report is never shown on this node, not even as history.
  const report = discovery?.node_id === nodeId ? discovery : undefined
  const current = report ? discoveryIsCurrent(report, nodeId, revision, usableRuntime) : false
  const listenersById = new Map(plan.listeners.map((listener) => [listener.id, listener]))

  const listeners: ListenerView[] = plan.listeners
    .filter((listener) => listener.node_id === nodeId)
    .map((listener) => ({
      id: listener.id,
      transport: listener.transport,
      bind: listener.bind,
      advertised: listener.advertised_endpoints.map(hostPort),
      runtime: usableRuntime?.listeners.find((observation) => observation.listener_id === listener.id),
      udp: udpFacts(listener.transport, "listener", listener.id, report, now, current),
      incoming: plan.candidates
        .filter((candidate) => candidate.listener_id === listener.id)
        .map((candidate) => ({
          candidateId: candidate.id,
          peerNodeId: candidate.dialer_node_id,
          endpoint: hostPort(candidate.endpoint),
          reports: candidateNeighbors(
            usableRuntime,
            { id: candidate.id, dialerNodeId: candidate.dialer_node_id, listenerId: listener.id, acceptorNodeId: nodeId, transport: listener.transport },
            nodeId
          ),
        })),
    }))

  const outgoing: OutgoingView[] = plan.candidates
    .filter((candidate) => candidate.dialer_node_id === nodeId)
    .sort((a, b) => a.priority - b.priority)
    .map((candidate) => {
      const listener = listenersById.get(candidate.listener_id)
      const transport = listener?.transport ?? null
      return {
        id: candidate.id,
        transport,
        targetNodeId: listener?.node_id ?? null,
        listenerId: candidate.listener_id,
        endpoint: hostPort(candidate.endpoint),
        priority: candidate.priority,
        udp: udpFacts(transport, "outgoing_candidate", candidate.id, report, now, current),
        check: {
          candidateId: candidate.id,
          peerNodeId: listener?.node_id ?? "",
          endpoint: hostPort(candidate.endpoint),
          reports: listener
            ? candidateNeighbors(
                usableRuntime,
                { id: candidate.id, dialerNodeId: nodeId, listenerId: listener.id, acceptorNodeId: listener.node_id, transport: listener.transport },
                nodeId
              )
            : [],
        },
      }
    })

  // Only UDP resources of the saved plan own reported UDP sockets; a resource
  // now on TCP / VLESS turns its old UDP socket into history.
  const known = new Set([
    ...listeners.filter((listener) => listener.transport === "native_udp").map((listener) => `listener:${listener.id}`),
    ...outgoing.filter((candidate) => candidate.transport === "native_udp").map((candidate) => `outgoing_candidate:${candidate.id}`),
  ])
  // Not a resource of the current plan: history, however recent the report.
  const orphans: OrphanSocket[] = report
    ? report.sockets
        .filter((socket) => !known.has(`${socket.resource_kind}:${socket.resource_id}`))
        .map((socket) => ({ kind: socket.resource_kind, resourceId: socket.resource_id, facts: socketFacts(report, socket, now, false) }))
    : []

  return { discoveryCurrent: current, listeners, outgoing, orphans }
}
