"use client"

import type { NodeSummary } from "@meshway/control-client"
import Link from "next/link"

import { Badge } from "@/components/ui/badge"
import { transportLabels } from "@/lib/connections-model"
import {
  channelStatusHints,
  channelStatusLabels,
  claimLabel,
  claimRtt,
  type Adjacency,
  type Channel,
  type ChannelStatus,
  type SideClaim,
} from "@/lib/topology-model"
import { cn } from "@/lib/utils"

const statusClass: Record<ChannelStatus, string> = {
  confirmed: "text-success",
  one_sided: "text-warning",
  degraded: "text-destructive",
  unobserved: "text-muted-foreground",
}

export function AdjacencyBadge({ status }: { status: ChannelStatus }) {
  return (
    <Badge variant="outline" className={cn("font-normal", statusClass[status])} title={channelStatusHints[status]}>
      {channelStatusLabels[status]}
    </Badge>
  )
}

export function NodeLink({ id, nodes }: { id: string; nodes: NodeSummary[] }) {
  const node = nodes.find((candidate) => candidate.id === id)
  return (
    <Link
      href={`/nodes/${encodeURIComponent(id)}`}
      className="min-w-0 truncate font-medium underline-offset-4 hover:underline"
      title={id}
    >
      {node?.name ?? `${id.slice(0, 12)}…`}
    </Link>
  )
}

function nameOf(nodes: NodeSummary[], id: string): string {
  return nodes.find((candidate) => candidate.id === id)?.name ?? `${id.slice(0, 12)}…`
}

/** One side's own probe, or why it has none; never a one-way latency. */
function SideValue({ claim }: { claim: SideClaim }) {
  const rtt = claimRtt(claim)
  if (rtt) return <span className="font-mono tabular-nums">{rtt}</span>
  return <span className={cn(claim.kind === "degraded" ? "text-destructive" : "text-muted-foreground")}>{claimLabel(claim)}</span>
}

export function channelText(channel: Channel, nodes: NodeSummary[]): string {
  const dialer = nameOf(nodes, channel.dialerNodeId)
  const acceptor = nameOf(nodes, channel.acceptorNodeId)
  const dialerRtt = claimRtt(channel.dialer) ?? claimLabel(channel.dialer)
  const acceptorRtt = claimRtt(channel.acceptor) ?? claimLabel(channel.acceptor)
  return `${transportLabels[channel.transport]}，${dialer} 发起，${channelStatusLabels[channel.status]}；${dialer} 侧 ${dialerRtt}，${acceptor} 侧 ${acceptorRtt}`
}

/** Channels of one pair: carrier, who dialed, and each side's own report. */
export function ChannelRows({
  adjacency,
  nodes,
  perspective,
}: {
  adjacency: Adjacency
  nodes: NodeSummary[]
  /** Node whose view this is ("本节点"), if any. */
  perspective?: string
}) {
  const label = (id: string) => (perspective === id ? "本节点" : nameOf(nodes, id))
  return (
    <ul className="flex flex-col gap-1.5">
      {adjacency.channels.map((channel) => (
        <li
          key={channel.candidateId}
          aria-label={channelText(channel, nodes)}
          className="flex flex-col gap-0.5 rounded-md bg-muted/40 px-2 py-1.5 text-[0.6875rem]"
        >
          <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
            <span className="font-medium">{transportLabels[channel.transport]}</span>
            <span className="text-muted-foreground">{label(channel.dialerNodeId)} 发起</span>
            <span className={cn("ml-auto", statusClass[channel.status])} title={channelStatusHints[channel.status]}>
              {channelStatusLabels[channel.status]}
            </span>
          </span>
          <span className="truncate font-mono text-muted-foreground" title={`候选 ${channel.candidateId} → 监听 ${channel.listenerId}`}>
            {channel.candidateId} → {channel.listenerId}
            {adjacency.channels.length > 1 ? ` · 优先级 ${channel.priority}` : ""}
          </span>
          <span className="grid grid-cols-[auto_1fr] gap-x-2 text-muted-foreground">
            <span>{label(channel.dialerNodeId)} 测得</span>
            <SideValue claim={channel.dialer} />
            <span>{label(channel.acceptorNodeId)} 测得</span>
            <SideValue claim={channel.acceptor} />
          </span>
        </li>
      ))}
    </ul>
  )
}

/** Text alternative to the map: one entry per configured node pair. */
export function AdjacencyList({
  adjacencies,
  nodes,
  isolated,
  selectedId,
  onSelect,
}: {
  adjacencies: Adjacency[]
  nodes: NodeSummary[]
  /** Active nodes without any configured adjacency. */
  isolated: NodeSummary[]
  selectedId: string | null
  onSelect: (id: string) => void
}) {
  return (
    <div className="flex flex-col gap-3 p-4">
      {adjacencies.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          已保存的连接配置中没有节点之间的邻接。
          <Link href="/connections" className="ml-1 underline underline-offset-4">
            连接配置
          </Link>
        </p>
      ) : (
        <ul aria-label="邻接列表" className="grid gap-2 md:grid-cols-2">
          {adjacencies.map((adjacency) => {
            const [a, b] = adjacency.nodeIds
            return (
              <li
                key={adjacency.key}
                data-testid="adjacency-summary-edge"
                data-status={adjacency.status}
                data-nodes={`${nameOf(nodes, a)}|${nameOf(nodes, b)}`}
                className={cn(
                  "flex min-w-0 flex-col gap-2 rounded-lg border px-3 py-2",
                  adjacency.nodeIds.includes(selectedId ?? "") && "border-brand/50"
                )}
              >
                <span className="flex min-w-0 items-center gap-1.5 text-xs">
                  <NodeLink id={a} nodes={nodes} />
                  <span aria-hidden="true" className="text-muted-foreground">
                    —
                  </span>
                  <span className="sr-only">与</span>
                  <NodeLink id={b} nodes={nodes} />
                  <span className="ml-auto shrink-0">
                    <AdjacencyBadge status={adjacency.status} />
                  </span>
                </span>
                {adjacency.ambiguousProof ? (
                  <span className="text-[0.6875rem] text-warning">
                    {nameOf(nodes, adjacency.channels[0]?.acceptorNodeId ?? "")} 报告了已认证通道，但无法确定对应哪个候选。
                  </span>
                ) : null}
                <ChannelRows adjacency={adjacency} nodes={nodes} />
                <span className="flex gap-2 text-[0.6875rem]">
                  {[a, b].map((id) => (
                    <button
                      key={id}
                      type="button"
                      className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                      onClick={() => onSelect(id)}
                    >
                      在图中选择 {nameOf(nodes, id)}
                    </button>
                  ))}
                </span>
              </li>
            )
          })}
        </ul>
      )}
      {isolated.length > 0 ? (
        <p data-testid="adjacency-isolated" className="text-xs text-muted-foreground">
          未配置邻接：
          {isolated.map((node, index) => (
            <span key={node.id}>
              {index > 0 ? "、" : ""}
              <NodeLink id={node.id} nodes={nodes} />
            </span>
          ))}
        </p>
      ) : null}
    </div>
  )
}
