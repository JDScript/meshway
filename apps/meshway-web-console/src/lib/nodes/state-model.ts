import type {
  AggregatedLink,
  AggregatedPath,
  ApplyPhase,
  ConfigurationState,
  EndpointCandidate,
  LinkAcquisition,
  ListenerState,
  NetmapRef,
  NodeState,
  NodeStateView,
  PeerState,
  SocketState,
  StateView,
} from "@meshway/control-client"

// Runtime State as the controller shows it to this account
// (docs/v2/state-reference): each node's last accepted report (`full`
// for the account's own or managed nodes, `peer` for the peers they
// depend on), when it arrived, and the controller's freshness. Only facts
// it gives; a timestamp past now is shown as expired, never as current.

/** A node's entry in the State; undefined when the State does not name it. */
export const stateOf = (
  state: StateView | undefined,
  nodeId: string
): NodeStateView | undefined =>
  state && Object.hasOwn(state.nodes, nodeId) ? state.nodes[nodeId] : undefined

export type Report =
  | { view: "full"; report: NodeState }
  | { view: "peer"; report: PeerState }
  | { view: "none" }

export function reportOf(view: NodeStateView | undefined): Report {
  const visible = view?.report
  if (!visible) return { view: "none" }
  return visible.view === "full"
    ? { view: "full", report: visible.report }
    : { view: "peer", report: visible.report }
}

/** The full report, when this account sees one. */
export const fullReport = (view: NodeStateView | undefined) => {
  const report = reportOf(view)
  return report.view === "full" ? report.report : null
}

/** `at` has passed. */
export const isPast = (at: string | null | undefined, now: number) =>
  at !== null && at !== undefined && Date.parse(at) <= now

/** An endpoint candidate past its expiry. */
export const candidateExpired = (candidate: EndpointCandidate, now: number) =>
  isPast(candidate.expiresAt, now)

/** The same per-node netmap content. */
export const sameNetmap = (a: NetmapRef, b: NetmapRef) =>
  a.serial === b.serial && a.digest === b.digest && a.revision === b.revision

/**
 * The netmap the server holds for the node: undefined while unread, null
 * when it holds none. A node is up to date when it applied exactly this one;
 * the network's revision says nothing about it (a revision that leaves the
 * node's netmap unchanged does not move it).
 */
export type Assigned = NetmapRef | null | undefined

/** What the node reports saving, applying and working towards. */
export type ConfigurationFacts = {
  saved: NetmapRef | null
  applied: NetmapRef | null
  /** The netmap the node is working towards, with its phase. */
  progress: { target: NetmapRef; phase: ApplyPhase } | null
  /** Applied the server's netmap for it (null: unknown, the server's is unread or none). */
  current: boolean | null
}

export function configurationFacts(
  configuration: ConfigurationState,
  assigned: Assigned
): ConfigurationFacts {
  const applied = configuration.applied ?? null
  return {
    saved: configuration.saved ?? null,
    applied,
    progress: configuration.progress
      ? {
          target: configuration.progress.target,
          phase: configuration.progress.phase,
        }
      : null,
    current:
      assigned == null
        ? null
        : applied !== null && sameNetmap(applied, assigned),
  }
}

/**
 * Where the node's configuration stands, as one fact: a netmap it failed to
 * apply or is applying, else the one it applied against the server's for it
 * (only the node's report, when the server's is unread or it holds none;
 * unknown without a full report).
 */
export type ConfigurationSync =
  | { kind: "failed" | "applying"; target: NetmapRef }
  | { kind: "current" | "applied"; applied: NetmapRef }
  | { kind: "behind"; applied: NetmapRef; assigned: NetmapRef }
  | { kind: "none" }
  | { kind: "unknown" }

export function configurationSync(
  configuration: ConfigurationState | null,
  assigned: Assigned
): ConfigurationSync {
  if (!configuration) return { kind: "unknown" }
  const { applied, progress } = configurationFacts(configuration, assigned)
  if (progress?.phase === "failed")
    return { kind: "failed", target: progress.target }
  if (progress && progress.phase !== "applied")
    return { kind: "applying", target: progress.target }
  if (applied === null) return { kind: "none" }
  if (assigned == null) return { kind: "applied", applied }
  return sameNetmap(applied, assigned)
    ? { kind: "current", applied }
    : { kind: "behind", applied, assigned }
}

/**
 * The server's netmap for the node against what the node reports saving
 * and applying. It is what the node should run, not what it runs: only the
 * node's own report says that.
 * - `none`: the server holds no netmap for the node;
 * - `unreported`: no full report to compare with;
 * - `applied` / `saved`: the node reports applying, or only saving, it;
 * - `elsewhere`: what the node reports saved and applied is another target
 *   (it has not taken this one yet, or not reported it).
 */
export type DesiredMatch =
  "none" | "unreported" | "applied" | "saved" | "elsewhere"

export function desiredAgainst(
  desired: NetmapRef | null,
  configuration: ConfigurationState | null
): DesiredMatch {
  if (desired === null) return "none"
  if (configuration === null) return "unreported"
  if (configuration.applied && sameNetmap(configuration.applied, desired))
    return "applied"
  if (configuration.saved && sameNetmap(configuration.saved, desired))
    return "saved"
  return "elsewhere"
}

/** A configured listener's runtime state and its sockets, as the full report has them. */
export type ListenerRuntime = {
  state: ListenerState
  sockets: { id: string; socket: SocketState | null }[]
}

export function listenerRuntime(
  report: NodeState,
  key: string
): ListenerRuntime | null {
  if (!Object.hasOwn(report.listeners, key)) return null
  const state = report.listeners[key]
  if (!state) return null
  return {
    state,
    sockets: state.socketIds.map((id) => ({
      id,
      socket: Object.hasOwn(report.sockets, id)
        ? (report.sockets[id] ?? null)
        : null,
    })),
  }
}

/** Sockets of the report no listener names: dial and coordination sockets. */
export function otherSockets(
  report: NodeState
): { id: string; socket: SocketState }[] {
  const named = new Set(
    Object.values(report.listeners).flatMap((listener) => listener.socketIds)
  )
  return Object.entries(report.sockets)
    .filter(([id]) => !named.has(id))
    .map(([id, socket]) => ({ id, socket }))
    .sort((a, b) => (a.id < b.id ? -1 : 1))
}

/** Listener keys the report has that the configuration does not (a node behind, or ahead). */
export const unconfiguredListeners = (
  report: NodeState,
  configured: readonly string[]
) =>
  Object.keys(report.listeners)
    .filter((key) => !configured.includes(key))
    .sort()

export type NodeLink = AggregatedLink & {
  peerId: string
  /** How this node acquired the channel, by its own report; null when it does not say (or this account does not see its report). */
  acquisition: LinkAcquisition | null
}

/** The node's links, each with the other end; by peer, then link ID. */
export function nodeLinks(
  state: StateView | undefined,
  nodeId: string
): NodeLink[] {
  const own = fullReport(stateOf(state, nodeId))
  return (state?.links ?? [])
    .filter((link) => link.a === nodeId || link.b === nodeId)
    .map((link) => {
      const peerId = link.a === nodeId ? link.b : link.a
      const reported =
        own && Object.hasOwn(own.links, peerId)
          ? own.links[peerId]?.find((item) => item.linkId === link.linkId)
          : undefined
      return { ...link, peerId, acquisition: reported?.acquisition ?? null }
    })
    .sort(
      (x, y) =>
        (x.peerId < y.peerId ? -1 : x.peerId > y.peerId ? 1 : 0) ||
        (x.linkId < y.linkId ? -1 : 1)
    )
}

/** The paths through the node, by path ID then reporter. */
export function nodePaths(
  state: StateView | undefined,
  nodeId: string
): AggregatedPath[] {
  return (state?.paths ?? [])
    .filter((path) => path.state.nodes.includes(nodeId))
    .sort(
      (x, y) =>
        (x.pathId < y.pathId ? -1 : x.pathId > y.pathId ? 1 : 0) ||
        (x.reporter.nodeId < y.reporter.nodeId ? -1 : 1)
    )
}
