import type {
  NetworkSummary,
  NodeInventoryResponse,
  NodeState,
  StateView,
} from "@meshway/control-client"
import { type UseQueryResult } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"

import { failureText } from "@/api/failure"
import { RefList, RefRow } from "@/components/chrome/ref-row"
import { EmptyLine, Section } from "@/components/chrome/section"
import { StatusDot } from "@/components/chrome/status-dot"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { dateTime, relativeTime } from "@/lib/format"
import {
  isPast,
  listenerRuntime,
  nodeLinks,
  nodePaths,
  otherSockets,
  reportOf,
  reportTime,
  stateOf,
  unconfiguredListeners,
} from "@/lib/nodes/state-model"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { useNodeConfiguration } from "../-draft"
import {
  evidenceLabel,
  freshnessLabel,
  linkPhaseLabel,
  pathEvidenceLabel,
  reporterRoleLabel,
  resourceText,
} from "../../-state-labels"
import { ConfiguredListeners } from "./listeners"
import { Runtime, SocketBlock } from "./runtime"

/** A time as when, with how long ago or until; "expired" once past. */
function TimeFact({
  at,
  now,
  expiry = false,
}: {
  at: string | null | undefined
  now: number
  expiry?: boolean
}) {
  if (!at) return <span className="text-muted-foreground">—</span>
  const past = isPast(at, now)
  return (
    <span className={cn(expiry && past && "text-warning")} title={at}>
      {expiry && past
        ? m.nodes_time_expired({ at: dateTime(at) ?? at })
        : (relativeTime(at, now) ?? at)}
    </span>
  )
}

/**
 * A node's transport: the listeners its configuration holds (edited here
 * into the draft) with what the node reports about each, the other sockets
 * it reports, and its links and paths as the controller aggregates them.
 */
export function NodeTransport({
  networkId,
  nodeId,
  network,
  state,
  nodes,
}: {
  networkId: string
  nodeId: string
  network: NetworkSummary | null
  state: UseQueryResult<StateView>
  nodes: NodeInventoryResponse | undefined
}) {
  const now = useNow(10_000)
  const configuration = useNodeConfiguration(networkId, nodeId, network)
  const view = stateOf(state.data, nodeId)
  const report = reportOf(view)
  const names = (id: string) => {
    const configured =
      configuration.state.kind === "manager"
        ? configuration.state.base.nodes[id]?.hostname
        : undefined
    return (
      configured ??
      nodes?.items.find((node) => node.id === id)?.hostname ??
      id.slice(0, 8)
    )
  }
  const full = report.view === "full" ? report.report : null

  return (
    <div className="flex flex-col gap-8">
      <ReportLine
        loading={state.data === undefined && !state.isError}
        failed={
          state.isError
            ? (failureText(state.error) ?? m.nodes_state_failed())
            : null
        }
        view={view}
        reportView={report.view}
        now={now}
      />
      <ConfiguredListeners
        nodeId={nodeId}
        configuration={configuration}
        report={full}
        now={now}
      />
      {full ? (
        <ReportedOnly
          report={full}
          configured={
            configuration.state.kind === "manager"
              ? Object.keys(
                  configuration.state.view.configuration.nodes[nodeId]
                    ?.listeners ?? {}
                )
              : configuration.state.kind === "member"
                ? Object.keys(configuration.state.settings.listeners)
                : null
          }
          now={now}
        />
      ) : null}
      {report.view === "peer" ? (
        <Section
          title={m.nodes_peer_endpoints()}
          count={report.report.endpoints.length}
        >
          {report.report.endpoints.length === 0 ? (
            <EmptyLine>{m.nodes_endpoints_none()}</EmptyLine>
          ) : (
            <ul className="flex flex-col gap-1">
              {report.report.endpoints.map((endpoint) => (
                <li
                  key={`${endpoint.socketId}-${endpoint.candidate.address}`}
                  className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 rounded-tile bg-fill-subtle px-3.5 py-2 text-row"
                >
                  <span className="truncate font-mono">
                    {endpoint.candidate.address}
                  </span>
                  <span className="text-meta text-muted-foreground">
                    {endpoint.network.toUpperCase()} ·{" "}
                    {resourceText(endpoint.resource)}
                  </span>
                  <span className="col-span-2 text-meta text-muted-foreground">
                    <TimeFact
                      at={endpoint.candidate.expiresAt}
                      now={now}
                      expiry
                    />
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Section>
      ) : null}
      <Links
        networkId={networkId}
        nodeId={nodeId}
        state={state.data}
        names={names}
        nodes={nodes}
        now={now}
      />
      <Paths nodeId={nodeId} state={state.data} names={names} now={now} />
    </div>
  )
}

function ReportLine({
  loading,
  failed,
  view,
  reportView,
  now,
}: {
  loading: boolean
  failed: string | null
  view: ReturnType<typeof stateOf>
  reportView: "full" | "peer" | "none"
  now: number
}) {
  if (loading) return <Skeleton className="h-12 w-full rounded-tile" />
  if (failed !== null) return <EmptyLine>{failed}</EmptyLine>
  if (!view) return <EmptyLine>{m.nodes_state_absent()}</EmptyLine>
  const at = reportTime(view)
  return (
    <div className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-1 rounded-tile bg-fill-subtle px-3.5 py-2 text-row">
      <StatusDot
        color={
          view.freshness === "current"
            ? "var(--vitality-live)"
            : "var(--vitality-quiet)"
        }
        shape={view.freshness === "current" ? "live" : "hollow"}
      />
      <span className="font-medium">{freshnessLabel[view.freshness]()}</span>
      <span className="text-muted-foreground">
        {at
          ? m.nodes_report_at({ at: relativeTime(at, now) ?? at })
          : m.nodes_report_none()}
      </span>
      <span className="ml-auto text-meta text-muted-foreground">
        {reportView === "full"
          ? m.nodes_report_full()
          : reportView === "peer"
            ? m.nodes_report_peer()
            : null}
      </span>
    </div>
  )
}

/** What the report has that no configured listener names: dial and coordination sockets, listeners not configured. */
function ReportedOnly({
  report,
  configured,
  now,
}: {
  report: NodeState
  /** The listener keys the current configuration holds; null when not readable. */
  configured: readonly string[] | null
  now: number
}) {
  const sockets = otherSockets(report)
  const extra =
    configured === null ? [] : unconfiguredListeners(report, configured)
  return (
    <>
      {extra.length > 0 ? (
        <Section title={m.nodes_listeners_unconfigured()} count={extra.length}>
          <div className="flex flex-col gap-2">
            {extra.map((key) => {
              const runtime = listenerRuntime(report, key)
              return runtime ? (
                <div key={key} className="flex flex-col gap-1">
                  <span className="px-1 font-mono text-row">{key}</span>
                  <Runtime runtime={runtime} now={now} />
                </div>
              ) : null
            })}
          </div>
        </Section>
      ) : null}
      <Section title={m.nodes_sockets_other()} count={sockets.length}>
        {sockets.length === 0 ? (
          <EmptyLine>{m.nodes_sockets_other_none()}</EmptyLine>
        ) : (
          <div className="flex flex-col gap-2">
            {sockets.map(({ id, socket }) => (
              <div key={id} className="flex flex-col gap-1">
                <span className="px-1 text-meta text-muted-foreground">
                  {resourceText(socket.resource)}
                </span>
                <SocketBlock id={id} socket={socket} now={now} />
              </div>
            ))}
          </div>
        )}
      </Section>
    </>
  )
}

function Links({
  networkId,
  nodeId,
  state,
  names,
  nodes,
  now,
}: {
  networkId: string
  nodeId: string
  state: StateView | undefined
  names: (id: string) => string
  nodes: NodeInventoryResponse | undefined
  now: number
}) {
  const navigate = useNavigate()
  const links = nodeLinks(state, nodeId)
  return (
    <Section title={m.nodes_links()} count={state ? links.length : null}>
      {state === undefined ? (
        <Skeleton className="h-12 w-full rounded-tile" />
      ) : links.length === 0 ? (
        <EmptyLine>{m.nodes_links_none()}</EmptyLine>
      ) : (
        <RefList>
          {links.map((link) => {
            const listed =
              nodes?.items.some((node) => node.id === link.peerId) ?? false
            return (
              <RefRow
                key={link.linkId}
                lead={
                  <StatusDot
                    color={
                      link.phase === "authenticated" &&
                      link.evidence === "matched"
                        ? "var(--vitality-live)"
                        : "var(--vitality-quiet)"
                    }
                    shape={
                      link.phase === "authenticated" &&
                      link.evidence === "matched"
                        ? "live"
                        : "hollow"
                    }
                  />
                }
                title={names(link.peerId)}
                subtitle={[
                  evidenceLabel[link.evidence](),
                  link.transport,
                  link.initiatorNodeId === nodeId
                    ? m.nodes_link_initiated_here()
                    : m.nodes_link_initiated_peer(),
                  link.inUse === true
                    ? m.nodes_link_in_use()
                    : link.inUse === false
                      ? m.nodes_link_idle()
                      : m.nodes_link_use_unknown(),
                  link.since
                    ? m.nodes_link_since({
                        at: relativeTime(link.since, now) ?? link.since,
                      })
                    : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
                state={linkPhaseLabel[link.phase]()}
                onOpen={
                  listed
                    ? () =>
                        void navigate({
                          to: "/networks/$network/nodes/$node",
                          params: { network: networkId, node: link.peerId },
                          search: { tab: "transport" },
                        })
                    : undefined
                }
              />
            )
          })}
        </RefList>
      )}
    </Section>
  )
}

function Paths({
  nodeId,
  state,
  names,
  now,
}: {
  nodeId: string
  state: StateView | undefined
  names: (id: string) => string
  now: number
}) {
  const paths = nodePaths(state, nodeId)
  if (state === undefined) return null
  return (
    <Section title={m.nodes_paths()} count={paths.length}>
      {paths.length === 0 ? (
        <EmptyLine>{m.nodes_paths_none()}</EmptyLine>
      ) : (
        <ul className="flex flex-col gap-1">
          {paths.map((path) => (
            <li
              key={`${path.pathId}-${path.reporter.nodeId}`}
              className="flex flex-col gap-0.5 rounded-tile bg-fill-subtle px-3.5 py-2"
            >
              <span className="truncate text-row font-medium">
                {path.state.nodes.map(names).join(" → ")}
              </span>
              <span className="text-meta text-muted-foreground">
                {[
                  pathEvidenceLabel[path.state.evidence](),
                  m.nodes_path_reported_by({
                    name: names(path.reporter.nodeId),
                    role: reporterRoleLabel[path.state.reporterRole](),
                  }),
                ].join(" · ")}
                {" · "}
                <TimeFact at={path.state.expiresAt} now={now} expiry />
              </span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  )
}
