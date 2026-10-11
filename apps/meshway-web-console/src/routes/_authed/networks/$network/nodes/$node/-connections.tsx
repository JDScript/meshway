import type {
  AdjacencyStatus,
  ListenerView,
  NetworkSummary,
  NodeInventoryResponse,
  NodeState,
  StateView,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import type { UseQueryResult } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import {
  ArrowUpRightIcon,
  CableIcon,
  ChevronRightIcon,
  RadioTowerIcon,
  WaypointsIcon,
} from "lucide-react"
import { useMemo, useState } from "react"

import { failureText } from "@/api/failure"
import { useAddressGeo } from "@/api/queries/ip-geo"
import { EmptyState } from "@/components/chrome/empty-state"
import { EmptyLine } from "@/components/chrome/section"
import { StatusDot } from "@/components/chrome/status-dot"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { agoTime } from "@/lib/format"
import { NO_ADDRESSES, nodeAddresses, type GeoOf } from "@/lib/geo"
import { millis } from "@/lib/history/format"
import { nodeConnections, type ConnectionStatus } from "@/lib/nodes/connections"
import {
  candidateExpired,
  listenerRuntime,
  nodeLinks,
  reportOf,
  stateOf,
} from "@/lib/nodes/state-model"
import {
  acquisitionText,
  connectionStatusLabel,
  endpointKindLabel,
  evidenceLabel,
  linkPhaseLabel,
  listenerPhaseLabel,
  transportText,
} from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import type { NodeTab } from "../-search"
import { linkParam } from "../../-map/selection"
import { useNodeConfiguration } from "./-draft"
import { Card, Disclosure, Row, Rows } from "./-kit"
import { useNeighborLatency } from "./-latency/neighbor-latency"
import { ReportedOnly } from "./-transport/reported-only"
import { EndpointGeo, Runtime } from "./-transport/runtime"

const STATUS_COLOR: Record<ConnectionStatus, string> = {
  connected: "var(--status-online)",
  connecting: "var(--status-attention)",
  failed: "var(--status-offline)",
  unknown: "var(--status-inactive)",
}

/**
 * How the node is connected: its direct neighbours (who, over what, since
 * when, how fast), and how other nodes reach it — its listeners and the
 * addresses they are reachable at. Sockets and candidates are one step in.
 */
export function NodeConnections({
  networkId,
  nodeId,
  network,
  state,
  nodes,
  nameOf,
  onTab,
}: {
  networkId: string
  nodeId: string
  network: NetworkSummary | null
  state: UseQueryResult<StateView>
  nodes: NodeInventoryResponse | undefined
  nameOf: (id: string) => string
  onTab: (tab: NodeTab) => void
}) {
  const now = useNow(1_000)
  const navigate = useNavigate()
  const view = stateOf(state.data, nodeId)
  const report = reportOf(view)
  const full = report.view === "full" ? report.report : null
  const addresses = useMemo(() => {
    const reported = stateOf(state.data, nodeId)
    return reported ? nodeAddresses(reported.derived).scopes : NO_ADDRESSES
  }, [state.data, nodeId])
  const geoOf = useAddressGeo(addresses)
  const latency = useNeighborLatency(networkId, nodeId)
  const configuration = useNodeConfiguration(networkId, nodeId, network)
  const settings = configuration.state
  const listeners: [string, ListenerView][] | null =
    configuration.state.kind === "manager"
      ? Object.entries(
          configuration.state.base.nodes[nodeId]?.listeners ?? {}
        ).sort(([a], [b]) => a.localeCompare(b))
      : configuration.state.kind === "member"
        ? Object.entries(configuration.state.settings.listeners).sort(
            ([a], [b]) => a.localeCompare(b)
          )
        : null
  const toTopology = (peer?: string) =>
    void navigate({
      to: "/networks/$network/topology",
      params: { network: networkId },
      search: {
        node: nodeId,
        link: peer ? linkParam(nodeId, peer) : undefined,
      },
    })

  const connections = nodeConnections(state.data?.links ?? [], nodeId)
  const channels = nodeLinks(state.data, nodeId)
  // Admitted peers it has no channel with: idle on demand, or unreachable.
  const unconnected = Object.entries(full?.adjacencies ?? {})
    .filter(([peer]) => !connections.some((item) => item.peerId === peer))
    .sort(
      ([a, x], [b, y]) =>
        Number(y === "unreachable") - Number(x === "unreachable") ||
        nameOf(a).localeCompare(nameOf(b))
    )

  return (
    <div className="flex flex-col gap-7">
      {state.isError && state.data !== undefined ? (
        <EmptyLine>
          {m.nodes_state_stale({
            reason: failureText(state.error) ?? m.nodes_state_failed(),
          })}
        </EmptyLine>
      ) : null}

      <Card
        title={m.nodes_neighbours()}
        aside={
          view?.report ? connections.length + unconnected.length : undefined
        }
      >
        {state.data === undefined && !state.isError ? (
          <Skeleton className="h-14 w-full rounded-tile" />
        ) : state.data === undefined ? (
          <EmptyLine>
            {failureText(state.error) ?? m.nodes_state_failed()}
          </EmptyLine>
        ) : !view?.report ? (
          <EmptyState icon={RadioTowerIcon} title={m.nodes_state_absent()}>
            {m.nodes_neighbours_unreported()}
          </EmptyState>
        ) : connections.length + unconnected.length === 0 ? (
          <EmptyState
            icon={CableIcon}
            title={m.nodes_neighbours_none()}
            action={
              <Button
                size="sm"
                variant="outline"
                className="rounded-full"
                onClick={() => toTopology()}
              >
                {m.nodes_neighbours_none_action()}
                <ArrowUpRightIcon />
              </Button>
            }
          >
            {m.nodes_neighbours_none_body()}
          </EmptyState>
        ) : (
          <Rows>
            {connections.map((connection) => (
              <Neighbour
                key={connection.key}
                name={nameOf(connection.peerId)}
                status={connection.status}
                transports={connection.transports}
                inUse={connection.channels.some(
                  (channel) => channel.inUse === true
                )}
                latency={
                  latency.loading
                    ? undefined
                    : (latency.byPeer.get(connection.peerId)?.meanUs ?? null)
                }
                channels={channels.filter(
                  (link) => link.peerId === connection.peerId
                )}
                nodeId={nodeId}
                nameOf={nameOf}
                now={now}
                listed={
                  nodes?.items.some((item) => item.id === connection.peerId) ??
                  false
                }
                onOpenPeer={() =>
                  void navigate({
                    to: "/networks/$network/nodes/$node",
                    params: { network: networkId, node: connection.peerId },
                    search: { tab: "connections" },
                  })
                }
                onTopology={() => toTopology(connection.peerId)}
              />
            ))}
            {unconnected.map(([peer, status]) => (
              <Unconnected
                key={peer}
                name={nameOf(peer)}
                status={status}
                onTopology={() => toTopology(peer)}
              />
            ))}
          </Rows>
        )}
      </Card>

      <Card
        title={m.nodes_reachable()}
        aside={listeners ? listeners.length : undefined}
      >
        {report.view === "peer" ? (
          report.report.endpoints.length === 0 ? (
            <EmptyState
              icon={RadioTowerIcon}
              title={m.nodes_endpoints_none()}
            />
          ) : (
            <Rows>
              {report.report.endpoints.map((endpoint) => (
                <Row
                  key={`${endpoint.socketId}-${endpoint.candidate.address}`}
                  label={endpoint.network.toUpperCase()}
                  mono
                  detail={endpointKindLabel[endpoint.candidate.source]()}
                >
                  {endpoint.candidate.address}
                </Row>
              ))}
            </Rows>
          )
        ) : settings.kind === "loading" ? (
          <Skeleton className="h-14 w-full rounded-tile" />
        ) : settings.kind === "failed" ? (
          <EmptyLine>
            {failureText(settings.error) ?? m.nodes_listeners_failed()}
          </EmptyLine>
        ) : settings.kind === "missing" ? (
          <EmptyLine>{m.nodes_listeners_missing()}</EmptyLine>
        ) : settings.kind === "unreadable" || listeners === null ? (
          <EmptyLine>{m.nodes_settings_unreadable()}</EmptyLine>
        ) : listeners.length === 0 ? (
          <EmptyState
            icon={RadioTowerIcon}
            title={m.nodes_reachable_none()}
            action={
              <Button
                size="sm"
                variant="outline"
                className="rounded-full"
                onClick={() => onTab("settings")}
              >
                {m.nodes_listener_add()}
              </Button>
            }
          >
            {m.nodes_reachable_none_body()}
          </EmptyState>
        ) : (
          <Rows>
            {listeners.map(([key, listener]) => (
              <Listener
                key={key}
                name={key}
                listener={listener}
                report={full}
                geoOf={geoOf}
                now={now}
              />
            ))}
          </Rows>
        )}
      </Card>

      {full ? (
        <Disclosure
          title={m.nodes_runtime_details()}
          summary={m.nodes_runtime_details_summary()}
        >
          {listeners?.map(([key]) => {
            const runtime = listenerRuntime(full, key)
            return runtime ? (
              <div key={key} className="flex flex-col gap-1.5">
                <span className="px-1 font-mono text-row">{key}</span>
                <Runtime runtime={runtime} geoOf={geoOf} now={now} />
              </div>
            ) : null
          })}
          <ReportedOnly
            report={full}
            geoOf={geoOf}
            configured={listeners?.map(([key]) => key) ?? null}
            now={now}
          />
        </Disclosure>
      ) : null}
    </div>
  )
}

/** An admitted neighbour with no channel now, as the node reports it; opens the link. */
function Unconnected({
  name,
  status,
  onTopology,
}: {
  name: string
  status: AdjacencyStatus
  onTopology: () => void
}) {
  return (
    <button
      type="button"
      title={m.nodes_link_in_topology()}
      onClick={onTopology}
      className="flex min-h-14 w-full items-center gap-3 px-4 py-2.5 text-left outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
    >
      <StatusDot
        color={
          status === "unreachable" ? STATUS_COLOR.failed : STATUS_COLOR.unknown
        }
        shape="hollow"
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-row font-medium">{name}</span>
        <span className="block truncate text-meta text-muted-foreground">
          {status === "unreachable"
            ? m.nodes_neighbour_unreachable()
            : m.nodes_neighbour_idle()}
        </span>
      </span>
      <ArrowUpRightIcon
        aria-hidden="true"
        className="size-4 shrink-0 text-muted-foreground"
      />
    </button>
  )
}

function Neighbour({
  name,
  status,
  transports,
  inUse,
  latency,
  channels,
  nodeId,
  nameOf,
  now,
  listed,
  onOpenPeer,
  onTopology,
}: {
  name: string
  status: ConnectionStatus
  transports: readonly string[]
  inUse: boolean
  /** undefined while the measurements are read; null when none succeeded. */
  latency: number | null | undefined
  channels: ReturnType<typeof nodeLinks>
  nodeId: string
  nameOf: (id: string) => string
  now: number
  listed: boolean
  onOpenPeer: () => void
  onTopology: () => void
}) {
  const [open, setOpen] = useState(false)
  const lead =
    channels.find((link) => link.phase === "authenticated") ?? channels[0]
  const how = lead
    ? lead.acquisition
      ? acquisitionText(lead.acquisition)
      : lead.initiatorNodeId === nodeId
        ? m.nodes_link_initiated_here()
        : m.nodes_link_initiated_peer()
    : null
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex min-h-14 w-full items-center gap-3 px-4 py-2.5 text-left outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      >
        <StatusDot
          color={STATUS_COLOR[status]}
          shape={status === "connected" ? "live" : "hollow"}
        />
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 items-center gap-2 text-row font-medium">
            <span className="truncate">{name}</span>
            {inUse ? (
              <span className="shrink-0 rounded-full bg-success/12 px-1.5 text-meta leading-4 font-medium text-success">
                {m.nodes_link_in_use()}
              </span>
            ) : null}
          </span>
          <span className="block truncate text-meta text-muted-foreground">
            {[
              connectionStatusLabel[status](),
              transports.map(transportText).join(" / ") || null,
              how,
              lead?.since
                ? m.nodes_link_since({ at: agoTime(lead.since, now) ?? "" })
                : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </span>
        {latency === undefined ? (
          <Skeleton className="h-2.5 w-10 shrink-0 rounded-full" />
        ) : (
          <span
            className={cn(
              "shrink-0 text-meta tabular-nums",
              latency === null ? "text-muted-foreground" : "text-foreground"
            )}
            title={m.nodes_tile_latency()}
          >
            {latency === null ? "—" : millis(latency)}
          </span>
        )}
        <ChevronRightIcon
          aria-hidden="true"
          className={cn(
            "size-4 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90"
          )}
        />
      </button>
      {open ? (
        <div className="flex flex-col gap-3 border-t border-divider bg-fill-subtle/50 px-4 py-3 pl-11">
          <ul className="flex flex-col gap-1 text-meta text-muted-foreground">
            {channels.map((link) => (
              <li key={link.linkId}>
                {[
                  transportText(link.transport),
                  linkPhaseLabel[link.phase](),
                  evidenceLabel[link.evidence](),
                  link.initiatorNodeId === nodeId
                    ? m.nodes_link_initiated_here()
                    : m.nodes_link_dialed_by({
                        name: nameOf(link.initiatorNodeId),
                      }),
                ].join(" · ")}
              </li>
            ))}
          </ul>
          <div className="flex flex-wrap gap-2">
            {listed ? (
              <Button
                size="sm"
                variant="outline"
                className="rounded-full"
                onClick={onOpenPeer}
              >
                {m.nodes_open_peer({ name })}
                <ArrowUpRightIcon />
              </Button>
            ) : null}
            <Button
              size="sm"
              variant="ghost"
              className="rounded-full"
              onClick={onTopology}
            >
              <WaypointsIcon />
              {m.nodes_link_in_topology()}
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  )
}

/** A listener in one row: what it listens on, whether it runs, and where it is reachable. */
function Listener({
  name,
  listener,
  report,
  geoOf,
  now,
}: {
  name: string
  listener: ListenerView
  report: NodeState | null
  geoOf: GeoOf
  now: number
}) {
  const runtime = report ? listenerRuntime(report, name) : null
  const reachable = (runtime?.sockets ?? []).flatMap(({ socket }) =>
    (socket?.endpoints ?? []).filter(
      (candidate) => !candidateExpired(candidate, now)
    )
  )
  const published = listener.endpoints.map(
    (endpoint) => `${endpoint.host}:${endpoint.port}`
  )
  const phase = runtime?.state.phase
  const first = reachable[0]
  return (
    <Row
      label={name}
      tone={!listener.enabled ? "muted" : undefined}
      detail={
        first ? (
          <span className="inline-flex min-w-0 items-baseline gap-1.5">
            <span className="font-mono">{first.address}</span>
            <span>{endpointKindLabel[first.origin.source]()}</span>
            <EndpointGeo address={first.address} geoOf={geoOf} />
            {reachable.length > 1
              ? m.nodes_tile_more({ count: reachable.length - 1 })
              : null}
          </span>
        ) : published.length ? (
          m.nodes_reachable_published({ value: published.join(", ") })
        ) : (
          m.nodes_reachable_unknown()
        )
      }
      action={
        <span
          className={cn(
            "shrink-0 text-meta",
            !listener.enabled
              ? "text-muted-foreground"
              : phase === "failed"
                ? "text-destructive"
                : phase === "listening"
                  ? "text-success"
                  : "text-muted-foreground"
          )}
        >
          {!listener.enabled
            ? m.nodes_listener_off()
            : phase
              ? listenerPhaseLabel[phase]()
              : report
                ? m.nodes_listener_unreported()
                : "—"}
        </span>
      }
    >
      {transportText(listener.protocol)}
      <span className="ml-2 font-mono text-[12px] text-muted-foreground">
        {listener.listen}:{listener.listenPort}
      </span>
    </Row>
  )
}
