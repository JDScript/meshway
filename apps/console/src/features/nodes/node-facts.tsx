import type {
  ConnectionRuntimesSnapshot,
  NodeSummary,
} from "@meshway/control-client"

import { cn } from "@/lib/utils"

import { CopyButton } from "@/components/chrome/copy-button"
import { RefList, RefRow, RefStateHeading } from "@/components/chrome/ref-row"
import { EmptyLine, Section } from "@/components/chrome/section"
import { StatusDot } from "@/components/chrome/status-dot"
import { Tile } from "@/components/chrome/tile"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { Vitality } from "@/features/globe/scene"
import {
  freshnessLabel,
  lookLabel,
  rttText,
  vitalityLabel,
} from "@/features/overview/labels"
import { lookOf, vitalityOf } from "@/features/overview/model"
import { LookDot, VitalityDot } from "@/features/overview/status-dots"
import { runtimeFor, runtimeFreshness } from "@/features/topology/runtime"
import {
  claimRttMs,
  nodeAdjacencies,
  otherEnd,
  type Channel,
  type TopologyObservation,
} from "@/features/topology/topology"
import { relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { transportLabel } from "./labels"

// The facts about one node, as pieces both the overview's peek and the
// node workspace lay out.

export type NodeContext = {
  nodes: ReadonlyMap<string, NodeSummary>
  observation: TopologyObservation | null
  runtimes: ConnectionRuntimesSnapshot | null
  now: number
}

/** A node's state on one line: its vitality, or that it is revoked (the dot's slot kept). */
export function VitalityLine({
  vitality,
  revoked = false,
}: {
  vitality: Vitality
  revoked?: boolean
}) {
  return (
    <span
      className={cn(
        "flex h-4 items-center gap-2 text-meta",
        revoked ? "text-warning" : "text-muted-foreground"
      )}
    >
      {revoked ? (
        <StatusDot color="var(--warning)" shape="hollow" />
      ) : (
        <VitalityDot vitality={vitality} />
      )}
      {revoked ? m.node_revoked() : vitalityLabel[vitality]()}
    </span>
  )
}

export const nodeVitality = (node: NodeSummary, context: NodeContext) =>
  vitalityOf(node, context.runtimes, context.now)

export function NodeAddresses({ node }: { node: NodeSummary }) {
  return (
    <div className="rounded-tile bg-fill-subtle py-2.5 pr-2 pl-3.5">
      {[node.mesh_ipv4, node.mesh_ipv6].map((address) => (
        <div
          key={address}
          className="flex h-8 items-center justify-between gap-2 font-mono text-row"
        >
          <span className="truncate">{address}</span>
          <CopyButton value={address} label={address} />
        </div>
      ))}
    </div>
  )
}

/** Report, contact, configuration receipt and version. */
export function NodeStatusTiles({
  node,
  context,
}: {
  node: NodeSummary
  context: NodeContext
}) {
  const report = runtimeFor(context.runtimes, node.id)
  const reportedAt = report?.reported_at
    ? relativeTime(report.reported_at, context.now)
    : null
  const pending = node.acknowledged_revision < node.configuration_revision
  return (
    <>
      <Tile label={m.node_report()}>
        {freshnessLabel[runtimeFreshness(report, context.now)]()}
        {reportedAt ? (
          <span className="font-normal text-muted-foreground">
            {" "}
            · {reportedAt}
          </span>
        ) : null}
      </Tile>
      <Tile label={m.node_contact()}>
        {relativeTime(node.last_seen, context.now) ?? m.node_never()}
      </Tile>
      <Tile label={m.node_receipt()} tone={pending ? "warning" : undefined}>
        <span className="font-mono">
          {pending
            ? m.node_receipt_pending({
                target: String(node.configuration_revision),
                acknowledged: String(node.acknowledged_revision),
              })
            : m.node_receipt_current({
                revision: String(node.acknowledged_revision),
              })}
        </span>
      </Tile>
      <Tile label={m.node_version()}>
        <span className="font-mono">{node.agent_version}</span>
      </Tile>
    </>
  )
}

/** A node's name, or the head of its ID for one the inventory does not hold. */
export const nodeName = (context: NodeContext, nodeId: string) =>
  context.nodes.get(nodeId)?.name ?? nodeId.slice(0, 12)

/** A channel's RTT as each end measured it, seen from `nodeId`; null when neither has. */
export function channelRtt(
  channel: Channel | undefined,
  nodeId: string
): { own: string | null; theirs: string | null } | null {
  if (!channel) return null
  const mine = channel.dialerNodeId === nodeId
  const own = rttText(claimRttMs(mine ? channel.dialer : channel.acceptor))
  const theirs = rttText(claimRttMs(mine ? channel.acceptor : channel.dialer))
  return own || theirs ? { own, theirs } : null
}

/** "12 ms · 14 ms": this end's measurement, then the other's. */
export function RttPair({
  rtt,
}: {
  rtt: { own: string | null; theirs: string | null } | null
}) {
  if (!rtt) return null
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span className="font-mono tabular-nums">
            {m.node_rtt_pair({
              own: rtt.own ?? "—",
              theirs: rtt.theirs ?? "—",
            })}
          </span>
        }
      />
      <TooltipContent>{m.node_rtt_hint()}</TooltipContent>
    </Tooltip>
  )
}

/** A node named from another's workspace (a tag's nodes, a member's devices); the row opens it. */
export function NodeRef({
  nodeId,
  context,
  onOpen,
}: {
  nodeId: string
  context: NodeContext
  onOpen: (nodeId: string) => void
}) {
  const node = context.nodes.get(nodeId)
  const vitality = node ? nodeVitality(node, context) : null
  return (
    <RefRow
      lead={vitality ? <VitalityDot vitality={vitality} /> : null}
      title={nodeName(context, nodeId)}
      detail={node?.mesh_ipv4 ?? ""}
      state={vitality ? vitalityLabel[vitality]() : null}
      onOpen={() => onOpen(nodeId)}
    />
  )
}

/** The node's configured channels with their evidence; a row peeks at the other end. */
export function NodeLinks({
  node,
  context,
  onOpenPeer,
}: {
  node: NodeSummary
  context: NodeContext
  onOpenPeer: (nodeId: string) => void
}) {
  const adjacencies = context.observation
    ? nodeAdjacencies(context.observation, node.id)
    : []
  return (
    <Section
      title={m.node_links()}
      count={adjacencies.length}
      action={
        adjacencies.length > 0 ? (
          <RefStateHeading withEnd={false}>
            {m.node_rtt_heading()}
          </RefStateHeading>
        ) : null
      }
    >
      {adjacencies.length === 0 ? (
        <EmptyLine>{m.node_links_none()}</EmptyLine>
      ) : (
        <RefList>
          {adjacencies.map((adjacency) => {
            const peerId = otherEnd(adjacency, node.id)
            const look = lookOf(adjacency.status)
            const channel = adjacency.channels[0]
            return (
              <RefRow
                key={adjacency.key}
                lead={<LookDot look={look} />}
                title={nodeName(context, peerId)}
                subtitle={
                  channel
                    ? `${transportLabel[channel.transport]()} · ${lookLabel[look]()}`
                    : lookLabel[look]()
                }
                state={<RttPair rtt={channelRtt(channel, node.id)} />}
                onSelect={() => onOpenPeer(peerId)}
              />
            )
          })}
        </RefList>
      )}
    </Section>
  )
}
