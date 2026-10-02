import type { ConnectionRuntimesSnapshot, ConnectionTransport, NodeSummary } from "@meshway/control-client"
import { XIcon } from "lucide-react"
import type { ReactNode } from "react"

import { CopyButton } from "@/components/chrome/copy-button"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { runtimeFor, runtimeFreshness } from "../topology/runtime"
import { claimRttMs, nodeAdjacencies, otherEnd, type TopologyObservation } from "../topology/topology"
import { freshnessLabel, lookColor, lookLabel, rttText, vitalityColor, vitalityLabel } from "./labels"
import { lookOf, vitalityOf } from "./model"

const transportLabel: Record<ConnectionTransport, string> = {
  native_udp: "UDP",
  native_tcp: "TCP",
  vless: "VLESS",
}

function Tile({ label, children, tone }: { label: string; children: ReactNode; tone?: "warning" }) {
  return (
    <div className="min-w-0 rounded-2xl bg-foreground/4 px-3.5 py-3">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className={tone === "warning" ? "mt-1 truncate text-[13px] font-medium text-warning" : "mt-1 truncate text-[13px] font-medium"}>
        {children}
      </div>
    </div>
  )
}

function Dot({ color, glow = false }: { color: string; glow?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className="inline-block size-2 shrink-0 rounded-full"
      style={{ background: color, boxShadow: glow ? `0 0 10px 1px ${color}` : undefined }}
    />
  )
}

export function NodeCard({
  node,
  nodes,
  observation,
  runtimes,
  now,
  onSelect,
}: {
  node: NodeSummary
  nodes: ReadonlyMap<string, NodeSummary>
  observation: TopologyObservation | null
  runtimes: ConnectionRuntimesSnapshot | null
  now: number
  onSelect: (nodeId: string | null) => void
}) {
  const vitality = vitalityOf(node, runtimes, now)
  const report = runtimeFor(runtimes, node.id)
  const freshness = runtimeFreshness(report, now)
  const reportedAt = report?.reported_at ? relativeTime(report.reported_at, now) : null
  const adjacencies = observation ? nodeAdjacencies(observation, node.id) : []
  const pending = node.acknowledged_revision < node.configuration_revision

  return (
    <section aria-labelledby="node-card-title" className="glass flex max-h-full flex-col overflow-hidden rounded-[28px]">
      <header className="flex items-start gap-3 px-6 pt-6 pb-5">
        <div className="min-w-0 flex-1">
          <h2 id="node-card-title" className="truncate text-xl font-semibold tracking-tight">
            {node.name}
          </h2>
          <p className="mt-1.5 flex items-center gap-2 text-xs text-muted-foreground">
            <Dot color={vitalityColor[vitality]} glow={vitality === "live"} />
            {vitalityLabel[vitality]()}
          </p>
        </div>
        <Button variant="ghost" size="icon-sm" className="-mt-1 -mr-2 rounded-full" aria-label={m.close()} onClick={() => onSelect(null)}>
          <XIcon />
        </Button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        <div className="rounded-2xl bg-foreground/4 px-3.5 py-2.5">
          {[node.mesh_ipv4, node.mesh_ipv6].map((address) => (
            <div key={address} className="flex h-8 items-center justify-between gap-2 font-mono text-[13px]">
              <span className="truncate">{address}</span>
              <CopyButton value={address} label={address} />
            </div>
          ))}
        </div>

        <div className="mt-2 grid grid-cols-2 gap-2">
          <Tile label={m.node_report()}>
            {freshnessLabel[freshness]()}
            {reportedAt ? <span className="font-normal text-muted-foreground"> · {reportedAt}</span> : null}
          </Tile>
          <Tile label={m.node_contact()}>{relativeTime(node.last_seen, now) ?? m.node_never()}</Tile>
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
        </div>

        <div className="mt-5 flex items-baseline justify-between px-2.5">
          <h3 className="text-xs text-muted-foreground">{m.node_links()}</h3>
          <span className="text-[11px] text-muted-foreground tabular-nums">{adjacencies.length > 0 ? m.node_rtt_heading() : null}</span>
        </div>
        {adjacencies.length === 0 ? (
          <p className="px-2.5 pt-2 text-[13px] text-muted-foreground">{m.node_links_none()}</p>
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
                    onClick={() => onSelect(peerId)}
                    className="flex w-full items-center gap-3 rounded-2xl px-2.5 py-2.5 text-left transition-colors duration-200 hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  >
                    <Dot color={lookColor[look]} glow={look === "confirmed"} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px]">{nodes.get(peerId)?.name ?? peerId.slice(0, 12)}</span>
                      <span className="block text-[11px] text-muted-foreground">
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
      </div>
    </section>
  )
}
