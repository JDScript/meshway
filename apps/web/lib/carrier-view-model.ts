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
  listeners: ListenerView[]
  outgoing: OutgoingView[]
  /** Reported sockets whose resource is not in the saved plan (e.g. removed since). */
  orphans: OrphanSocket[]
}

const hostPort = (endpoint: { host: string; port: number }) =>
  endpoint.host.includes(":") && !endpoint.host.startsWith("[") ? `[${endpoint.host}]:${endpoint.port}` : `${endpoint.host}:${endpoint.port}`

export function observedState(report: NodeEndpointDiscovery, observation: StunEndpointObservation, now: number): ObservedState {
  if (observationUsable(report, observation, now)) return "candidate"
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

function socketFacts(report: NodeEndpointDiscovery, socket: UdpSocketDiscovery, now: number): SocketFacts {
  return {
    boundAddress: socket.bound_address,
    generation: socket.generation,
    socketId: socket.socket_id,
    source: socket.source,
    observed: socket.observations.map((observation) => ({
      address: observation.mapped_endpoint,
      observer: observation.observer,
      outcome: observation.outcome,
      state: observedState(report, observation, now),
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
  now: number
): UdpFacts {
  if (transport !== "native_udp") return { kind: "not_applicable" }
  const sockets = report?.sockets.filter((socket) => socket.resource_kind === kind && socket.resource_id === id) ?? []
  if (!report || sockets.length === 0) return { kind: "none" }
  return { kind: "sockets", sockets: sockets.map((socket) => socketFacts(report, socket, now)) }
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
  const usableRuntime = reportMatchesBase(runtime, revision, now) ? runtime : undefined
  const listenersById = new Map(plan.listeners.map((listener) => [listener.id, listener]))

  const listeners: ListenerView[] = plan.listeners
    .filter((listener) => listener.node_id === nodeId)
    .map((listener) => ({
      id: listener.id,
      transport: listener.transport,
      bind: listener.bind,
      advertised: listener.advertised_endpoints.map(hostPort),
      runtime: usableRuntime?.listeners.find((observation) => observation.listener_id === listener.id),
      udp: udpFacts(listener.transport, "listener", listener.id, discovery, now),
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
        udp: udpFacts(transport, "outgoing_candidate", candidate.id, discovery, now),
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

  const known = new Set([
    ...listeners.map((listener) => `listener:${listener.id}`),
    ...outgoing.map((candidate) => `outgoing_candidate:${candidate.id}`),
  ])
  const orphans: OrphanSocket[] = discovery
    ? discovery.sockets
        .filter((socket) => !known.has(`${socket.resource_kind}:${socket.resource_id}`))
        .map((socket) => ({ kind: socket.resource_kind, resourceId: socket.resource_id, facts: socketFacts(discovery, socket, now) }))
    : []

  return { listeners, outgoing, orphans }
}
