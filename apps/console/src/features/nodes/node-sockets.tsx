import type {
  ConnectionsSnapshot,
  ManagedListenerSummary,
  UdpSocketDiscovery,
} from "@meshway/control-client"

import { EmptyLine } from "@/components/chrome/section"
import { Skeleton } from "@/components/ui/skeleton"
import { relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import {
  discoveryStatus,
  observationState,
  reportUsable,
  socketsByResource,
  type discoveryFor,
} from "./discovery-model"
import {
  discoverySourceLabel,
  discoveryStatusLabel,
  observationLabel,
  stunOutcomeLabel,
  transportLabel,
} from "./labels"
import type { NodeContext } from "./node-facts"

/**
 * The node's actual sockets as its endpoint discovery reports them, with
 * each dialing socket tied to the peer its candidate names. Listeners with
 * no such report (TCP and VLESS) are listed as unknown, never inferred from
 * their saved settings or from UDP observations.
 */
export function NodeSockets({
  listeners,
  connections,
  discoveryRead,
  discoveryFailed,
  context,
  nodeId,
  discovery,
}: {
  listeners: ManagedListenerSummary[]
  connections: ConnectionsSnapshot | null
  discoveryRead: boolean
  discoveryFailed: boolean
  context: NodeContext
  nodeId: string
  discovery: ReturnType<typeof discoveryFor>
}) {
  if (!discoveryRead)
    return discoveryFailed ? (
      <EmptyLine>{m.node_discovery_failed()}</EmptyLine>
    ) : (
      <Skeleton className="h-16 w-full rounded-tile" />
    )
  const status = discoveryStatus(discovery)
  const usable = reportUsable(status)
  const { listeners: listening, dialers } = socketsByResource(
    usable ? discovery : undefined
  )
  const peerOf = (candidateId: string) => {
    const candidate = connections?.configuration.candidates.find(
      (entry) => entry.id === candidateId && entry.dialer_node_id === nodeId
    )
    const listener = connections?.configuration.listeners.find(
      (entry) => entry.id === candidate?.listener_id
    )
    return listener ? (context.nodes.get(listener.node_id)?.name ?? null) : null
  }
  const unknown = listeners.filter(
    (listener) =>
      listener.settings.enabled && listener.settings.transport !== "native_udp"
  )
  const sockets = [...listening.values(), ...dialers.values()].flat()

  return (
    <div className="flex flex-col gap-2">
      {sockets.length === 0 ? (
        <EmptyLine>
          {usable ? m.node_sockets_none() : discoveryStatusLabel[status]()}
        </EmptyLine>
      ) : (
        <ul className="flex flex-col gap-2">
          {sockets.map((socket) => (
            <SocketItem
              key={`${socket.generation}/${socket.socket_id}`}
              socket={socket}
              peer={
                socket.resource_kind === "outgoing_candidate"
                  ? peerOf(socket.resource_id)
                  : null
              }
              usable={usable}
              now={context.now}
            />
          ))}
        </ul>
      )}
      {unknown.length > 0 ? (
        <ul className="flex flex-col gap-2">
          {unknown.map((listener) => (
            <li
              key={listener.settings.id}
              className="flex h-12 items-center gap-3 rounded-tile bg-fill-subtle px-3.5 text-row"
            >
              <span className="text-meta text-muted-foreground">
                {m.node_socket_listener()}
              </span>
              <span className="truncate font-mono">{listener.settings.id}</span>
              <span className="text-meta text-muted-foreground">
                {transportLabel[listener.settings.transport]()}
              </span>
              <span className="flex-1" />
              <span className="text-meta text-muted-foreground">
                {m.node_socket_unknown()}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}

function SocketItem({
  socket,
  peer,
  usable,
  now,
}: {
  socket: UdpSocketDiscovery
  peer: string | null
  usable: boolean
  now: number
}) {
  const dial = socket.resource_kind === "outgoing_candidate"
  return (
    <li className="flex flex-col gap-1 rounded-tile bg-fill-subtle px-3.5 py-2.5">
      <div className="flex items-baseline gap-2">
        <span className="text-meta text-muted-foreground">
          {dial ? m.node_socket_dial() : m.node_socket_listener()}
        </span>
        <span className="truncate font-mono text-row font-medium">
          {socket.resource_id}
        </span>
        {dial ? (
          <span className="truncate text-meta text-muted-foreground">
            → {peer ?? m.node_unknown()}
          </span>
        ) : null}
        <span className="flex-1" />
        <span className="shrink-0 text-meta text-muted-foreground">
          {discoverySourceLabel[socket.source]()}
        </span>
      </div>
      <div className="flex flex-wrap gap-x-4 font-mono text-meta text-muted-foreground">
        <span>
          {m.node_socket_bound()} {socket.bound_address}
        </span>
        <span>
          {m.node_socket_ordinal({
            generation: String(socket.generation),
            socket: String(socket.socket_id),
          })}
        </span>
      </div>
      {socket.observations.length === 0 ? (
        <div className="pt-1.5">
          <EmptyLine size="sm" dashed>
            {m.node_observations_none()}
          </EmptyLine>
        </div>
      ) : (
        // One grid for every observation (each row a subgrid), so the columns line up.
        <ul className="mt-1 grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 border-t border-divider sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
          {socket.observations.map((observation) => {
            const state = observationState(observation, usable, now)
            return (
              <li
                key={observation.observer}
                className="col-span-full grid grid-cols-subgrid items-center gap-y-0.5 py-2"
              >
                <span className="truncate font-mono text-row">
                  {state === "mapped" || state === "expired"
                    ? (observation.mapped_endpoint ??
                      stunOutcomeLabel[observation.outcome]())
                    : stunOutcomeLabel[observation.outcome]()}
                </span>
                <span
                  className={
                    state === "mapped"
                      ? "text-right text-meta sm:order-last"
                      : "text-right text-meta text-muted-foreground sm:order-last"
                  }
                >
                  {observationLabel[state]()}
                  {observation.rtt_ms !== null
                    ? ` · ${observation.rtt_ms} ms`
                    : ""}
                </span>
                <span className="col-span-2 truncate text-meta text-muted-foreground sm:col-span-1">
                  <span className="font-mono">{observation.observer}</span>
                  {" · "}
                  {m.node_observation_times({
                    observed: relativeTime(observation.observed_at, now) ?? "—",
                    expires: relativeTime(observation.expires_at, now) ?? "—",
                  })}
                </span>
              </li>
            )
          })}
        </ul>
      )}
    </li>
  )
}
