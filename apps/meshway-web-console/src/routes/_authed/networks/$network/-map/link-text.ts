import type { MapConnection } from "@/lib/network-map"
import { connectionStatusLabel, transportText } from "@/lib/state-labels"
import type { DiagramEdge } from "@/lib/topology-diagram"
import { m } from "@/paraglide/messages.js"

/** Carriers in their order, the first preferred. */
export const orderText = (transports: readonly string[]) =>
  transports.map(transportText).join(" → ")

/** A link's state: what the nodes report, else that it is not connected (or cannot be). */
export function linkStateText(connection: MapConnection | null): string {
  switch (connection?.status ?? "allowed") {
    case "allowed":
      return connection?.adjacency?.status === "idle"
        ? m.map_link_adjacency_idle()
        : connection?.adjacency?.status === "unreachable"
          ? m.map_link_adjacency_unreachable()
          : m.map_link_idle()
    case "unavailable":
      return m.map_link_no_carrier()
    case "history":
      return m.overview_history()
    case "connected":
      return connectionStatusLabel.connected()
    case "connecting":
      return connectionStatusLabel.connecting()
    case "failed":
      return connectionStatusLabel.failed()
    default:
      return connectionStatusLabel.unknown()
  }
}

/**
 * The carrier a link shows: the one in use while connected, else the
 * configuration's restriction when it has one. Nothing for an edge that
 * takes the platform's order and is not connected.
 */
export function carrierText(connection: MapConnection | null): string | null {
  if (connection?.status === "connected" && connection.transports.length > 0)
    return connection.transports.map(transportText).join(" / ")
  const allowed = connection?.declaration?.transports
  if (allowed == null) return null
  if (allowed.length === 0) return m.map_link_no_carrier()
  return m.map_link_only({ value: orderText(allowed) })
}

/** A link's one line under its name in a list. */
export function linkSummary(connection: MapConnection | null): string {
  return [
    linkStateText(connection),
    carrierText(connection),
    connection?.inUse ? m.map_link_in_use() : null,
  ]
    .filter(Boolean)
    .join(" · ")
}

export const edgeCaption = (edge: DiagramEdge) => carrierText(edge.connection)

/** Everything an edge says, for its accessible name and tooltip. */
export function edgeTitle(
  edge: DiagramEdge,
  nameOf: (id: string) => string
): string {
  const ends = edge.direction
    ? `${nameOf(edge.direction.from)} → ${nameOf(edge.direction.to)}`
    : `${nameOf(edge.a)} ↔ ${nameOf(edge.b)}`
  return [
    ends,
    linkSummary(edge.connection),
    edge.unauthorized ? m.map_route_unauthorized() : null,
  ]
    .filter(Boolean)
    .join(" · ")
}
