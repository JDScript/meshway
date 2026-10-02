import type { ConnectionRuntimesSnapshot, NodeSummary } from "@meshway/control-client"
import { XIcon } from "lucide-react"
import type { ReactNode } from "react"

import { CopyButton } from "@/components/chrome/copy-button"
import { Button } from "@/components/ui/button"
import { relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { runtimeFor, runtimeFreshness } from "../topology/runtime"
import { claimRttMs, nodeAdjacencies, otherEnd, type TopologyObservation } from "../topology/topology"
import { freshnessLabel, lookColor, lookLabel, rttText, vitalityColor, vitalityLabel } from "./labels"
import { lookOf, vitalityOf } from "./model"

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-2.5">
      <dt className="shrink-0 text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate text-right text-[13px]">{children}</dd>
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
  const acknowledged = node.acknowledged_revision >= node.configuration_revision

  return (
    <section aria-labelledby="node-card-title" className="glass flex h-full flex-col overflow-hidden rounded-[28px]">
      <header className="flex items-start gap-3 px-6 pt-6">
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

      <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-5 pb-6">
        <div className="rounded-2xl bg-foreground/4 px-4 py-3">
          <div className="text-xs text-muted-foreground">{m.node_mesh_address()}</div>
          {[node.mesh_ipv4, node.mesh_ipv6].map((address) => (
            <div key={address} className="mt-1.5 flex items-center justify-between gap-2 font-mono text-[13px]">
              <span className="truncate">{address}</span>
              <CopyButton value={address} label={address} />
            </div>
          ))}
        </div>

        <dl className="mt-3 divide-y divide-foreground/6">
          <Row label={m.node_report()}>
            {freshnessLabel[freshness]()}
            {reportedAt ? <span className="text-muted-foreground"> · {reportedAt}</span> : null}
          </Row>
          <Row label={m.node_contact()}>{relativeTime(node.last_seen, now) ?? m.node_never()}</Row>
          <Row label={m.node_receipt()}>
            <span className={acknowledged ? "font-mono" : "font-mono text-warning"}>
              {acknowledged
                ? m.node_receipt_current({ revision: String(node.acknowledged_revision) })
                : m.node_receipt_pending({ target: String(node.configuration_revision), acknowledged: String(node.acknowledged_revision) })}
            </span>
          </Row>
          <Row label={m.node_version()}>
            <span className="font-mono">{node.agent_version}</span>
          </Row>
          <Row label={m.node_location()}>{m.node_location_unknown()}</Row>
        </dl>

        <h3 className="mt-6 mb-2 text-xs text-muted-foreground">{m.node_links()}</h3>
        {adjacencies.length === 0 ? (
          <p className="text-[13px] text-muted-foreground">{m.node_links_none()}</p>
        ) : (
          <ul className="-mx-2 flex flex-col">
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
                    className="flex w-full items-center gap-3 rounded-xl px-2 py-2.5 text-left transition-colors duration-200 hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  >
                    <Dot color={lookColor[look]} glow={look === "confirmed"} />
                    <span className="min-w-0 flex-1 truncate text-[13px]">{nodes.get(peerId)?.name ?? peerId.slice(0, 12)}</span>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {own || theirs ? (
                        <span className="font-mono tabular-nums">{m.node_rtt_pair({ own: own ?? "—", theirs: theirs ?? "—" })}</span>
                      ) : (
                        lookLabel[look]()
                      )}
                    </span>
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
