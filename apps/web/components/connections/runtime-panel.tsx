"use client"

import type { ConnectionNode, ConnectionRuntimeObservation } from "@meshway/control-client"
import { ActivityIcon, CircleAlertIcon, RefreshCwIcon } from "lucide-react"
import type { ReactNode } from "react"

import type { Runtimes } from "@/components/connections/use-runtimes"
import { Panel } from "@/components/common/panel"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  formatRtt,
  freshnessHints,
  freshnessLabels,
  neighborCounts,
  neighborLabel,
  reasonLabels,
  runtimeFor,
  runtimeFreshness,
  runtimeSummary,
  type RuntimeFreshness,
} from "@/lib/connection-runtime-model"
import { nodeName, transportLabels } from "@/lib/connections-model"
import { formatClock, formatDateTime, formatRelative } from "@/lib/time"
import { cn } from "@/lib/utils"

const freshnessClass: Record<RuntimeFreshness, string> = {
  fresh: "text-success",
  unreported: "text-muted-foreground",
  awaiting: "text-muted-foreground",
  withdrawn: "text-muted-foreground",
  outdated: "text-warning",
  expired: "text-warning",
}

export function FreshnessBadge({ freshness }: { freshness: RuntimeFreshness }) {
  return (
    <Badge variant="outline" className={cn("font-normal", freshnessClass[freshness])} title={freshnessHints[freshness]}>
      {freshnessLabels[freshness]}
    </Badge>
  )
}

/** One-line runtime state for overview rows. */
export function RuntimeChip({
  observation,
  now,
  readFailed,
}: {
  observation: ConnectionRuntimeObservation | undefined
  now: number
  readFailed: boolean
}) {
  if (readFailed && !observation) {
    return <span className="text-xs text-muted-foreground">运行状态读取失败</span>
  }
  const freshness = runtimeFreshness(observation, now)
  if (freshness !== "fresh" || !observation) return <FreshnessBadge freshness={freshness} />
  const counts = neighborCounts(observation)
  const total = observation.peers.length
  return (
    <span className="inline-flex min-w-0 flex-wrap items-center gap-1.5 text-xs" title={freshnessHints.fresh}>
      <Badge
        variant="outline"
        className={cn("font-normal", observation.state === "error" ? "text-destructive" : observation.state === "applied" ? "text-success" : "")}
      >
        {runtimeSummary(observation)}
      </Badge>
      {total > 0 ? (
        <span className="text-muted-foreground tabular-nums">
          {counts.authenticated}/{total} 邻居已认证
        </span>
      ) : null}
    </span>
  )
}

/**
 * The agent's own report for one node, kept apart from the editable plan:
 * a background refresh replaces it, the draft below never moves.
 */
export function NodeRuntimePanel({
  nodeId,
  nodes,
  runtimes,
}: {
  nodeId: string
  nodes: ConnectionNode[]
  runtimes: Runtimes
}) {
  const observation = runtimeFor(runtimes.snapshot, nodeId)
  const freshness = runtimeFreshness(observation, runtimes.now)
  const history = freshness !== "fresh"

  return (
    <Panel
      data-testid="connection-runtime"
      data-freshness={runtimes.snapshot === null ? "unavailable" : freshness}
      title={
        <span className="flex flex-wrap items-center gap-2">
          <ActivityIcon className="size-4 text-muted-foreground" aria-hidden="true" />
          实际连接状态
          {runtimes.snapshot !== null ? <FreshnessBadge freshness={freshness} /> : null}
        </span>
      }
      description="节点 agent 自报的监听与已认证邻居。只说明节点之间的认证通道，不代表路由或 TUN 流量已放行。"
      actions={
        <span className="flex items-center gap-1.5">
          {runtimes.fetchedAt !== null ? (
            <span className="hidden text-xs text-muted-foreground tabular-nums sm:inline">
              读取于 {formatClock(runtimes.fetchedAt)}
            </span>
          ) : null}
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={runtimes.refresh}
            disabled={runtimes.refreshing}
            aria-label="刷新运行状态"
          >
            <RefreshCwIcon className={runtimes.refreshing ? "animate-spin" : undefined} />
          </Button>
        </span>
      }
      bodyClassName="flex flex-col gap-3 p-4"
    >
      {runtimes.error ? (
        <Alert variant={runtimes.snapshot ? "default" : "destructive"}>
          <CircleAlertIcon />
          <AlertTitle>{runtimes.snapshot ? "最近一次刷新失败，以下为上次读取的结果" : "无法读取运行状态"}</AlertTitle>
          <AlertDescription className="break-all">{runtimes.error}</AlertDescription>
        </Alert>
      ) : null}

      {runtimes.snapshot === null ? null : !observation || freshness === "unreported" ? (
        <p className="text-xs text-muted-foreground">
          {freshnessHints.unreported} 节点以托管连接模式运行（
          <code className="font-mono">run --apply-connections</code>）后才会报告。
        </p>
      ) : (
        <>
          <p className={cn("text-xs", history ? "text-warning" : "text-muted-foreground")}>{freshnessHints[freshness]}</p>
          <dl className={cn("grid gap-x-6 gap-y-2 text-xs sm:grid-cols-3", history && "opacity-70")}>
            <Fact label="运行阶段">
              <span className={cn(observation.state === "error" && "text-destructive")}>{runtimeSummary(observation)}</span>
            </Fact>
            <Fact label="报告时间">
              {observation.reported_at ? (
                <span title={formatDateTime(observation.reported_at)}>
                  {formatRelative(observation.reported_at, runtimes.now) ?? "—"}
                </span>
              ) : (
                "尚未报告"
              )}
            </Fact>
            <Fact label="对应配置">
              <span className="font-mono tabular-nums">
                {observation.revision === null ? "—" : `r${observation.revision}`}
              </span>
              <span className="text-muted-foreground">{observation.current ? " · 当前版本" : " · 非当前版本"}</span>
            </Fact>
          </dl>

          {observation.listeners.length > 0 ? (
            <div className={cn("flex flex-col gap-1.5", history && "opacity-70")}>
              <h3 className="text-xs font-medium">本机监听</h3>
              <ul className="flex flex-wrap gap-1.5">
                {observation.listeners.map((listener) => (
                  <li key={listener.listener_id}>
                    <Badge
                      variant="outline"
                      className={cn("max-w-full font-normal", listener.state === "error" ? "text-destructive" : "")}
                    >
                      <span className="truncate font-mono">{listener.listener_id}</span>
                      <span className="text-muted-foreground">
                        {listener.state === "bound" ? "已监听" : reasonLabels[listener.reason] || "错误"}
                      </span>
                    </Badge>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}

          <div className={cn("flex flex-col gap-1.5", history && "opacity-70")}>
            <h3 className="text-xs font-medium">邻居</h3>
            {observation.peers.length === 0 ? (
              <p className="text-xs text-muted-foreground">报告中没有邻居。</p>
            ) : (
              <div className="rounded-lg border text-xs">
                <div
                  aria-hidden="true"
                  className={cn(peerGrid, "hidden border-b bg-muted/40 py-2 font-medium text-muted-foreground sm:grid")}
                >
                  <span>邻居</span>
                  <span>承载</span>
                  <span>谁发起</span>
                  <span>状态</span>
                  <span className="text-right">探测 RTT</span>
                </div>
                <ul aria-label="已报告的邻居" className="divide-y">
                  {observation.peers.map((peer) => {
                    const name = nodeName(nodes, peer.peer_node_id)
                    const initiator = peer.initiator_node_id === nodeId ? "本节点发起" : "对方发起"
                    const rtt = formatRtt(peer)
                    return (
                      <li
                        key={`${peer.peer_node_id}|${peer.transport}|${peer.initiator_node_id}|${peer.listener_id ?? ""}|${peer.candidate_id ?? ""}`}
                        data-testid="connection-runtime-peer"
                        data-peer-id={peer.peer_node_id}
                        data-state={peer.state}
                        aria-label={`${name}：${transportLabels[peer.transport]}，${initiator}，${neighborLabel(peer)}，RTT ${rtt}`}
                        className={cn(peerGrid, "py-2.5")}
                      >
                        <span className="col-start-1 row-start-1 min-w-0 truncate font-medium">{name}</span>
                        {/* Mobile: one secondary line under the name; desktop: own columns. */}
                        <span className="col-span-2 row-start-2 flex min-w-0 flex-wrap gap-x-2 text-muted-foreground sm:contents">
                          <span className="sm:text-foreground">{transportLabels[peer.transport]}</span>
                          <span className="min-w-0 [overflow-wrap:anywhere]">
                            <span className="sm:hidden">· </span>
                            {initiator}
                            {peer.listener_id ? <span className="ml-1 font-mono text-[0.6875rem]">→ {peer.listener_id}</span> : null}
                          </span>
                        </span>
                        <span className="col-start-2 row-start-1 flex items-center justify-end gap-2 sm:contents">
                          <span
                            className={cn(
                              "text-right sm:text-left",
                              peer.state === "authenticated" && !history && "text-success",
                              peer.state === "unreachable" && "text-destructive"
                            )}
                          >
                            {neighborLabel(peer)}
                          </span>
                          <span className="text-right font-mono tabular-nums">{rtt}</span>
                        </span>
                      </li>
                    )
                  })}
                </ul>
              </div>
            )}
            <p className="text-[0.6875rem] text-muted-foreground">
              「已认证」表示 agent 最近完成了一次邻居认证探测；RTT 仅来自该探测。
            </p>
          </div>
        </>
      )}
    </Panel>
  )
}

/** Mobile: name + status on one line, details below; desktop: five columns. */
const peerGrid =
  "grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-3 gap-y-0.5 px-3 sm:grid-cols-[minmax(0,9rem)_6rem_minmax(0,1.6fr)_minmax(0,1fr)_4.5rem] sm:gap-y-0"

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate">{children}</dd>
    </div>
  )
}
