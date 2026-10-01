"use client"

import type { ConnectionNode, ConnectionRuntimeObservation } from "@meshway/control-client"
import { ActivityIcon, RefreshCwIcon } from "lucide-react"
import type { ReactNode } from "react"

import type { Runtimes } from "@/components/connections/use-runtimes"
import { Panel } from "@/components/common/panel"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { FailureNotice } from "@/components/common/failure-notice"
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
  runtimeNextSteps,
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
       description="来源：Agent 自报的监听与邻居认证探测 · 业务授权另由 Access 决定。"
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
        <FailureNotice title={runtimes.snapshot ? "刷新失败，显示上次读取的运行报告" : "无法读取运行状态"} nextStep="点击刷新运行状态重试；持续失败时检查控制面与节点联系。" error={runtimes.error} />
      ) : null}

      {runtimes.snapshot === null ? null : !observation || freshness === "unreported" ? (
        <p className="text-xs text-muted-foreground">
          尚无运行报告。查看此节点的控制面联系；当前启用参数可在节点高级帮助中找到。
        </p>
      ) : (
        <>
          {history ? <p className="text-xs text-warning">{freshnessHints[freshness]}</p> : null}
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
            <Fact label="邻居认证探测">{neighborCounts(observation).authenticated} / {observation.peers.length} 已认证{history ? "（历史报告）" : ""}</Fact>
          </dl>

          {observation.reason !== "none" ? <p role="status" className="text-xs text-warning">{runtimeNextSteps[observation.reason]}</p> : null}

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
              {observation.listeners.some((listener) => listener.reason !== "none") ? (
                <ul aria-label="监听问题下一步" className="flex flex-col gap-1 text-xs text-warning">
                  {[...new Set(observation.listeners.filter((listener) => listener.reason !== "none").map((listener) => listener.reason))].map((reason) => <li key={reason}>{runtimeNextSteps[reason]}</li>)}
                </ul>
              ) : null}
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
            {observation.peers.length > 0 ? <p className="text-[0.6875rem] text-muted-foreground">
              「已认证」表示 agent 最近完成了一次邻居认证探测；RTT 仅来自该探测。
            </p> : null}
          </div>
          {observation.peers.some((peer) => peer.reason !== "none") ? <ul aria-label="连接问题下一步" className="flex flex-col gap-1 text-xs text-warning">{observation.peers.filter((peer) => peer.reason !== "none").map((peer, index) => <li key={index}>{nodeName(nodes, peer.peer_node_id)}：{runtimeNextSteps[peer.reason]}</li>)}</ul> : null}
          <DiagnosticDetails title="运行报告技术详情">
            <p>对应配置 r{observation.revision ?? "—"} · current {String(observation.current)} · stale {String(observation.stale)} · 有效期 {observation.stale_after_seconds} 秒</p>
            <p>运行阶段 {observation.state} · 原因 {observation.reason}</p>
            <ul className="flex flex-col gap-1">{observation.peers.map((peer, index) => <li key={index}>{nodeName(nodes, peer.peer_node_id)} · listener {peer.listener_id ?? "—"} · candidate {peer.candidate_id ?? "—"} · initiator {peer.initiator_node_id}</li>)}</ul>
          </DiagnosticDetails>
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
