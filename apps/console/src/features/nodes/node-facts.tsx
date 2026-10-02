import type { ConnectionRuntimesSnapshot, ConnectionTransport, NodeSummary } from "@meshway/control-client"
import { cn } from "cn"
import type { ReactNode } from "react"

import { CopyButton } from "@/components/chrome/copy-button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import type { Vitality } from "@/features/globe/scene"
import { freshnessLabel, lookLabel, rttText, vitalityLabel } from "@/features/overview/labels"
import { lookOf, vitalityOf } from "@/features/overview/model"
import { LookDot, VitalityDot } from "@/features/overview/status-dots"
import { runtimeFor, runtimeFreshness } from "@/features/topology/runtime"
import { claimRttMs, nodeAdjacencies, otherEnd, type TopologyObservation } from "@/features/topology/topology"
import { relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

// The facts about one node, as pieces both the overview's peek and the
// node workspace lay out.

export type NodeContext = {
  nodes: ReadonlyMap<string, NodeSummary>
  observation: TopologyObservation | null
  runtimes: ConnectionRuntimesSnapshot | null
  now: number
}

const transportLabel: Record<ConnectionTransport, string> = {
  native_udp: "UDP",
  native_tcp: "TCP",
  vless: "VLESS",
}

export function VitalityLine({ vitality }: { vitality: Vitality }) {
  return (
    <span className="flex items-center gap-2 text-xs text-muted-foreground">
      <VitalityDot vitality={vitality} />
      {vitalityLabel[vitality]()}
    </span>
  )
}

export const nodeVitality = (node: NodeSummary, context: NodeContext) => vitalityOf(node, context.runtimes, context.now)

/** One labelled fact; `action` sits at its end (a copy button). */
export function Tile({ label, children, tone, action }: { label: string; children: ReactNode; tone?: "warning"; action?: ReactNode }) {
  return (
    <div className="flex min-w-0 items-center gap-1 rounded-tile bg-fill-subtle py-3 pr-2 pl-3.5">
      <div className="min-w-0 flex-1">
        <div className="text-meta text-muted-foreground">{label}</div>
        <div className={cn("mt-1 truncate text-row font-medium", tone === "warning" && "text-warning")}>{children}</div>
      </div>
      {action}
    </div>
  )
}

export function NodeAddresses({ node }: { node: NodeSummary }) {
  return (
    <div className="rounded-tile bg-fill-subtle py-2.5 pr-2 pl-3.5">
      {[node.mesh_ipv4, node.mesh_ipv6].map((address) => (
        <div key={address} className="flex h-8 items-center justify-between gap-2 font-mono text-row">
          <span className="truncate">{address}</span>
          <CopyButton value={address} label={address} />
        </div>
      ))}
    </div>
  )
}

/** Report, contact, configuration receipt and version. */
export function NodeStatusTiles({ node, context }: { node: NodeSummary; context: NodeContext }) {
  const report = runtimeFor(context.runtimes, node.id)
  const reportedAt = report?.reported_at ? relativeTime(report.reported_at, context.now) : null
  const pending = node.acknowledged_revision < node.configuration_revision
  return (
    <>
      <Tile label={m.node_report()}>
        {freshnessLabel[runtimeFreshness(report, context.now)]()}
        {reportedAt ? <span className="font-normal text-muted-foreground"> · {reportedAt}</span> : null}
      </Tile>
      <Tile label={m.node_contact()}>{relativeTime(node.last_seen, context.now) ?? m.node_never()}</Tile>
      <Tile label={m.node_receipt()} tone={pending ? "warning" : undefined}>
        <span className="font-mono">
          {pending
            ? m.node_receipt_pending({ target: String(node.configuration_revision), acknowledged: String(node.acknowledged_revision) })
            : m.node_receipt_current({ revision: String(node.acknowledged_revision) })}
        </span>
      </Tile>
      <Tile label={m.node_version()}>
        <span className="font-mono">{node.agent_version}</span>
      </Tile>
    </>
  )
}

/** The node's configured channels with their evidence; a row opens the other end. */
export function NodeLinks({ node, context, onOpenPeer }: { node: NodeSummary; context: NodeContext; onOpenPeer: (nodeId: string) => void }) {
  const adjacencies = context.observation ? nodeAdjacencies(context.observation, node.id) : []
  return (
    <section className="-mx-2.5">
      <div className="flex items-baseline justify-between px-2.5">
        <h3 className="text-xs text-muted-foreground">{m.node_links()}</h3>
        <span className="text-meta text-muted-foreground tabular-nums">{adjacencies.length > 0 ? m.node_rtt_heading() : null}</span>
      </div>
      {adjacencies.length === 0 ? (
        <p className="px-2.5 pt-2 text-row text-muted-foreground">{m.node_links_none()}</p>
      ) : (
        <ul className="mt-1 flex flex-col">
          {adjacencies.map((adjacency) => {
            const peerId = otherEnd(adjacency, node.id)
            const look = lookOf(adjacency.status)
            const channel = adjacency.channels[0]
            const own = channel ? rttText(claimRttMs(channel.dialerNodeId === node.id ? channel.dialer : channel.acceptor)) : null
            const theirs = channel ? rttText(claimRttMs(channel.dialerNodeId === node.id ? channel.acceptor : channel.dialer)) : null
            return (
              <li key={adjacency.key}>
                <button
                  type="button"
                  onClick={() => onOpenPeer(peerId)}
                  className="list-row gap-3 px-2.5 py-2.5"
                >
                  <LookDot look={look} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-row">{context.nodes.get(peerId)?.name ?? peerId.slice(0, 12)}</span>
                    <span className="block text-meta text-muted-foreground">
                      {channel ? `${transportLabel[channel.transport]} · ` : null}
                      {lookLabel[look]()}
                    </span>
                  </span>
                  {own || theirs ? (
                    <Tooltip>
                      <TooltipTrigger
                        render={
                          <span className="shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
                            {m.node_rtt_pair({ own: own ?? "—", theirs: theirs ?? "—" })}
                          </span>
                        }
                      />
                      <TooltipContent>{m.node_rtt_hint()}</TooltipContent>
                    </Tooltip>
                  ) : null}
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
