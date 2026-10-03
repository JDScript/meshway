import type {
  EndpointDiscoveriesSnapshot,
  EndpointDiscoveryState,
  NodeEndpointDiscovery,
  StunEndpointObservation,
  UdpSocketDiscovery,
} from "@meshway/control-client"

// What an agent's actual managed UDP sockets observed through STUN
// (docs/endpoint-discovery.md). A mapping is that socket's observation for
// its runtime generation, valid until it expires; it is not proof that a
// peer can reach it, and a UDP mapping says nothing about TCP / VLESS.
// Listener sockets and dial (outgoing candidate) sockets stay apart.

/**
 * The node's discovery report as a whole: its own state, unless the
 * controller marks it as not current (another configuration) or stale.
 */
export type DiscoveryStatus =
  "unreported" | "outdated" | "stale" | EndpointDiscoveryState

export function discoveryFor(
  snapshot: EndpointDiscoveriesSnapshot | undefined,
  nodeId: string
): NodeEndpointDiscovery | undefined {
  return snapshot?.nodes.find((entry) => entry.node_id === nodeId)
}

export function discoveryStatus(
  entry: NodeEndpointDiscovery | undefined
): DiscoveryStatus {
  if (!entry) return "unreported"
  if (entry.state === "withdrawn" || entry.state === "awaiting_report")
    return entry.state
  if (!entry.current) return "outdated"
  if (entry.stale) return "stale"
  return entry.state
}

/** Whether the report can stand for the present at all. */
export const reportUsable = (status: DiscoveryStatus) =>
  status === "observed" || status === "probing"

/**
 * - `mapped`: a mapped endpoint, before its expiry, from a usable report.
 * - `expired`: past its expiry, or the report no longer stands.
 * - `failed`: the observer did not map (timeout, invalid response, I/O, STUN error).
 */
export type ObservationState = "mapped" | "expired" | "failed"

export function observationState(
  observation: StunEndpointObservation,
  usable: boolean,
  now: number
): ObservationState {
  const expires = Date.parse(observation.expires_at)
  if (!usable || Number.isNaN(expires) || now >= expires) return "expired"
  if (observation.outcome !== "mapped" || observation.mapped_endpoint === null)
    return "failed"
  return "mapped"
}

type SocketGroups = {
  listeners: Map<string, UdpSocketDiscovery[]>
  dialers: Map<string, UdpSocketDiscovery[]>
}

/** Sockets by the controller resource they serve: a listener or an outgoing candidate. */
export function socketsByResource(
  entry: NodeEndpointDiscovery | undefined
): SocketGroups {
  const listeners = new Map<string, UdpSocketDiscovery[]>()
  const dialers = new Map<string, UdpSocketDiscovery[]>()
  for (const socket of entry?.sockets ?? []) {
    const groups = socket.resource_kind === "listener" ? listeners : dialers
    groups.set(socket.resource_id, [
      ...(groups.get(socket.resource_id) ?? []),
      socket,
    ])
  }
  return { listeners, dialers }
}
