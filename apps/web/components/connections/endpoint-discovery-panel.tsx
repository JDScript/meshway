"use client"

import { useRoutes } from "@/components/shell/network-context"
import { useNetworkId } from "@/components/network/network-workspace"

import type { EndpointDiscoveriesSnapshot, NodeEndpointDiscovery, UdpSocketDiscovery } from "@meshway/control-client"
import Link from "next/link"
import { useCallback, useEffect, useRef, useState, useTransition } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { FailureNotice } from "@/components/common/failure-notice"
import { Panel } from "@/components/common/panel"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { ControllerResult } from "@/lib/controller"
import { readEndpointDiscoveriesAction } from "@/lib/endpoint-discovery-actions"
import { discoveryLabel, discoveryOutcomeLabels, observationUsable } from "@/lib/endpoint-discovery-model"
import { advanceDiscoveryClock, shownDiscoveryRead, type DiscoveryClock } from "@/lib/endpoint-discovery-view-model"
import { formatDateTime } from "@/lib/time"

export type EndpointDiscoveries = {
  shown: ControllerResult<EndpointDiscoveriesSnapshot>
  /** Monotonic display clock: an expired lease never becomes usable again. */
  clock: number
  refreshing: boolean
  refresh: () => void
}

/** Polled discovery reports with a clock that keeps leases ageing between reads. */
export function useEndpointDiscoveries(initial: ControllerResult<EndpointDiscoveriesSnapshot>): EndpointDiscoveries {
  const [read, setRead] = useState({ initial, result: initial })
  const networkId = useNetworkId()
  const [now, setNow] = useState<DiscoveryClock>({ time: initial.fetchedAt, monotonicTime: null })
  const [refreshing, startRefresh] = useTransition()
  const sequence = useRef(0)
  if (read.initial !== initial) {
    setRead({ initial, result: initial })
    // Preserve every displayed clock floor, including prop replacements that
    // arrive between ticks. A subsequent older server time cannot undo expiry.
    setNow((previous) => ({ ...previous, time: Math.max(previous.time, initial.fetchedAt) }))
  }
  const shown = shownDiscoveryRead(initial, read)
  const sampleClock = useCallback((floor = initial.fetchedAt) => {
    const wallTime = Math.max(Date.now(), floor)
    const monotonicTime = performance.now()
    setNow((previous) => advanceDiscoveryClock(previous, wallTime, monotonicTime))
  }, [initial.fetchedAt])
  const refresh = useCallback(() => {
    const seq = ++sequence.current
    startRefresh(async () => {
      const result = await readEndpointDiscoveriesAction(networkId)
      if (seq !== sequence.current) return
      // A failed read replaces the snapshot: never present last-good as live.
      setRead({ initial, result })
      sampleClock(result.fetchedAt)
    })
  }, [initial, sampleClock, networkId])
  useEffect(() => {
    const tick = window.setInterval(() => sampleClock(), 1_000)
    const poll = window.setInterval(() => { if (document.visibilityState === "visible") refresh() }, 12_000)
    const visible = () => {
      sampleClock()
      if (document.visibilityState === "visible") refresh()
    }
    document.addEventListener("visibilitychange", visible)
    const owned = sequence
    return () => {
      owned.current += 1
      window.clearInterval(tick)
      window.clearInterval(poll)
      document.removeEventListener("visibilitychange", visible)
    }
  }, [refresh, sampleClock])
  return { shown, clock: Math.max(now.time, shown.fetchedAt), refreshing, refresh }
}

export function EndpointDiscoveryPanel({ initial, nodeId }: {
  initial: ControllerResult<EndpointDiscoveriesSnapshot>
  nodeId?: string
}) {
  return <EndpointDiscoveryReport discoveries={useEndpointDiscoveries(initial)} nodeId={nodeId} />
}

/** The raw report for one node: state, sockets, observer outcomes and technical fields. */
export function EndpointDiscoveryReport({ discoveries, nodeId, embedded = false, fenced = true, now }: {
  discoveries: EndpointDiscoveries
  nodeId?: string
  /** Inside the node's carrier view: no link back to the node page. */
  embedded?: boolean
  /** False when the caller's local fence (plan revision, runtime epoch / digest) rejects the report: history only. */
  fenced?: boolean
  /** A caller's shared monotonic clock; the view never shows a lease the caller already expired. */
  now?: number
}) {
  const routes = useRoutes()
  const { shown, refreshing, refresh } = discoveries
  const clock = Math.max(discoveries.clock, now ?? 0)
  const report = shown.ok && nodeId ? shown.data.nodes.find((node) => node.node_id === nodeId) : undefined

  return <Panel title="UDP 承载地址观测" description="来源：Agent 的实际承载 socket · 地址尚未验证可达。" actions={<Button size="sm" variant="outline" disabled={refreshing} onClick={refresh}>{refreshing ? "读取中…" : "刷新观测"}</Button>} bodyClassName="flex min-w-0 flex-col gap-3 px-4 py-3 text-xs">
    <div data-testid="endpoint-discovery" className="flex min-w-0 flex-col gap-3">
      {!shown.ok ? <FailureNotice title="端点观测读取失败" nextStep="点击刷新观测重试；持续失败时检查控制面服务。" error={shown.error} /> : !nodeId ? <p className="text-muted-foreground">选择源节点，查看它实际 UDP 承载的 STUN 地址观测。</p> : <>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline" data-testid="discovery-status">{!fenced && report?.state === "observed" && report.reported_at !== null ? "与当前配置 / 运行代际不一致" : discoveryLabel(report, clock)}</Badge>
          {embedded ? null : <Link href={routes.machine(nodeId, "connections")} className="underline underline-offset-4">节点连接配置与运行状态</Link>}
        </div>
        {report?.reported_at ? <p className="text-muted-foreground">上报于 {formatDateTime(report.reported_at)}</p> : null}
        {report ? <DiagnosticDetails title="报告技术详情"><dl className="grid min-w-0 gap-x-4 gap-y-1 sm:grid-cols-2">
          <div className="sm:col-span-2">state：{report.state} · epoch {report.epoch || "未知"} · sequence {report.sequence}</div>
          <div><dt className="inline">配置 revision：</dt><dd className="inline">{report.revision === null ? "未知" : `r${report.revision}`}</dd></div>
          <div><dt className="inline">Controller current / stale：</dt><dd className="inline">{String(report.current)} / {String(report.stale)}</dd></div>
          <div className="min-w-0 sm:col-span-2"><dt className="inline">digest：</dt><dd className="inline"><code className="break-all">{report.digest ?? "未知"}</code></dd></div>
          <div className="sm:col-span-2"><dt className="inline">上报于：</dt><dd className="inline">{report.reported_at === null ? "从未（无报告）" : formatDateTime(report.reported_at)}</dd></div>
        </dl></DiagnosticDetails> : null}
        {report?.reported_at !== null && report?.state === "observed" && (!report.current || report.stale || !fenced) ? <p role="status" className="text-warning">此报告已不属于当前配置 / 运行代际，或租约已失效；地址仅为历史观测。</p> : null}
        {!report || report.sockets.length === 0 ? <div className="flex flex-col gap-2 text-muted-foreground">
          <p>{report?.state === "withdrawn" || report?.state === "expired" ? "观测已撤回或过期，地址已清除。刷新以获取新报告。" : report?.state === "disabled" ? "此节点尚未启用 STUN 地址观测。" : report?.state === "probing" ? "正在等待承载地址探测结果，请稍后刷新。" : "尚无地址报告。查看节点运行状态，或展开当前启用帮助。"}</p>
          <DiagnosticDetails title="当前 STUN 启用帮助"><p>在此节点现有 <code>run</code> 启动参数中显式加入 <code className="break-all">--stun-server &lt;NUMERIC_IP&gt;:3478</code>（IPv6 使用 <code>[&lt;NUMERIC_IPV6&gt;]:3478</code>）；沿用原有 --state、--apply-connections / --apply-dataplane 和 --connection-profiles。重新应用现有启动配置，不另起第二个 daemon。此处是当前启用说明，不会通过 Web 修改启动参数。</p></DiagnosticDetails>
        </div> : report.sockets.map((socket) => <SocketReport key={`${socket.resource_kind}:${socket.resource_id}:${socket.generation}:${socket.socket_id}`} socket={socket} report={report} now={clock} fenced={fenced} />)}
      </>}
      <p className="text-muted-foreground">来源 managed_udp_socket_stun；与独立 netcheck、手工发布地址分开。观测不会自动拨号或更改配置。</p>
    </div>
  </Panel>
}

function SocketReport({ socket, report, now, fenced }: { socket: UdpSocketDiscovery; report: NodeEndpointDiscovery; now: number; fenced: boolean }) {
  const historical = !fenced || !report.current || report.stale || report.state === "withdrawn" || report.state === "expired" ||
    (socket.observations.length > 0 && socket.observations.every((observation) => Date.parse(observation.expires_at) <= now))
  return <section aria-label={`承载地址 ${socket.resource_id}`} className="flex min-w-0 flex-col gap-2 rounded-lg border p-3">
      <h3 className="font-medium break-all">{socket.resource_kind === "listener" ? "接收连接" : "主动连接"} · {socket.resource_id}</h3>
      <Address label={historical ? "历史本地绑定 IP:port（失效）" : "本地绑定 IP:port"} value={socket.bound_address} disabled={historical} />
      <DiagnosticDetails title="承载 socket 技术详情"><p>generation {socket.generation} / socket {socket.socket_id}</p><p className="break-all text-muted-foreground">source：{socket.source}</p></DiagnosticDetails>
      {socket.observations.length === 0 ? <p className="text-muted-foreground">此 socket 尚无 observer 结果。</p> : socket.observations.map((observation) => {
        const usable = fenced && observationUsable(report, observation, now)
        const expired = Date.parse(observation.expires_at) <= now
        return <div key={`${observation.observer}:${observation.observed_at}`} data-testid="discovery-observation" className="flex min-w-0 flex-col gap-1.5 border-t pt-2">
          <div className="flex flex-wrap gap-2"><Badge variant="outline">{usable ? "未验证候选观测" : expired ? "已过期 · 不可使用" : discoveryOutcomeLabels[observation.outcome]}</Badge>{!usable && observation.mapped_endpoint !== null && !expired ? <span className="text-warning">已失效 / 非当前观测</span> : null}</div>
          {observation.mapped_endpoint === null ? <p>观测端点：无映射结果</p> : <Address label={usable ? "观测 IP:port" : "历史观测 IP:port（失效）"} value={observation.mapped_endpoint} disabled={!usable} />}
          <p className="break-all text-muted-foreground">有效期至 {formatDateTime(observation.expires_at)} · {expired ? "已过期" : usable ? "到期后不可使用" : "不可使用"}</p>
          <DiagnosticDetails title="STUN 探测详情">
            <Address label="STUN observer" value={observation.observer} />
            <p className="text-muted-foreground">STUN RTT：{observation.rtt_ms === null ? "未测得" : `${observation.rtt_ms} ms`}（不是 peer RTT）</p>
            <p className="break-all text-muted-foreground">observed_at：{formatDateTime(observation.observed_at)} · expires_at：{formatDateTime(observation.expires_at)}</p>
          </DiagnosticDetails>
        </div>
      })}
  </section>
}

function Address({ label, value, disabled = false }: { label: string; value: string; disabled?: boolean }) {
  return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-start gap-2"><div className="min-w-0"><span className="text-muted-foreground">{label}：</span><code className="break-all">{value}</code></div><CopyButton value={value} label={`复制${label}`} iconOnly disabled={disabled} /></div>
}
