"use client"

import type { EndpointDiscoveriesSnapshot, NodeEndpointDiscovery, UdpSocketDiscovery } from "@meshway/control-client"
import Link from "next/link"
import { useCallback, useEffect, useRef, useState, useTransition } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { Panel } from "@/components/common/panel"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { ControllerResult } from "@/lib/controller"
import { readEndpointDiscoveriesAction } from "@/lib/endpoint-discovery-actions"
import { discoveryLabel, discoveryOutcomeLabels, observationUsable } from "@/lib/endpoint-discovery-model"
import { formatDateTime } from "@/lib/time"

export function EndpointDiscoveryPanel({ initial, nodeId }: {
  initial: ControllerResult<EndpointDiscoveriesSnapshot>
  nodeId?: string
}) {
  const [read, setRead] = useState(initial)
  const [now, setNow] = useState(initial.fetchedAt)
  const [refreshing, startRefresh] = useTransition()
  const sequence = useRef(0)
  const shown = initial.fetchedAt > read.fetchedAt ? initial : read
  const refresh = useCallback(() => {
    const seq = ++sequence.current
    startRefresh(async () => {
      const result = await readEndpointDiscoveriesAction()
      if (seq !== sequence.current) return
      // A failed read replaces the snapshot: never present last-good as live.
      setRead(result)
      setNow(Date.now())
    })
  }, [])
  useEffect(() => {
    const tick = window.setInterval(() => setNow(Date.now()), 1_000)
    const poll = window.setInterval(() => { if (document.visibilityState === "visible") refresh() }, 12_000)
    const visible = () => {
      setNow(Date.now())
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
  }, [refresh])
  const report = shown.ok && nodeId ? shown.data.nodes.find((node) => node.node_id === nodeId) : undefined
  const clock = Math.max(now, shown.fetchedAt)

  return <Panel title="UDP carrier 端点观测" description="Agent 经节点身份认证的自报；未验证候选观测，不证明可达、peer 身份或连接成功。" actions={<Button size="sm" variant="outline" disabled={refreshing} onClick={refresh}>{refreshing ? "读取中…" : "刷新观测"}</Button>} bodyClassName="flex min-w-0 flex-col gap-3 px-4 py-3 text-xs">
    <div data-testid="endpoint-discovery" className="flex min-w-0 flex-col gap-3">
      {!shown.ok ? <div role="alert" className="text-destructive">端点观测读取失败：{shown.error}。请刷新重试。</div> : !nodeId ? <p className="text-muted-foreground">选择源节点，查看它实际 UDP carrier socket 的 STUN 观测。</p> : <>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline" data-testid="discovery-status">{discoveryLabel(report, clock)}</Badge>
          {report ? <span className="text-muted-foreground">state：{report.state} · epoch {report.epoch || "未知"} · sequence {report.sequence}</span> : null}
          <Link href={`/nodes/${encodeURIComponent(nodeId)}?tab=connections`} className="underline underline-offset-4">节点连接配置与运行状态</Link>
        </div>
        {report ? <dl className="grid min-w-0 gap-x-4 gap-y-1 sm:grid-cols-2">
          <div><dt className="inline">配置 revision：</dt><dd className="inline">{report.revision === null ? "未知" : `r${report.revision}`}</dd></div>
          <div><dt className="inline">Controller current / stale：</dt><dd className="inline">{String(report.current)} / {String(report.stale)}</dd></div>
          <div className="min-w-0 sm:col-span-2"><dt className="inline">digest：</dt><dd className="inline"><code className="break-all">{report.digest ?? "未知"}</code></dd></div>
          <div className="sm:col-span-2"><dt className="inline">上报于：</dt><dd className="inline">{report.reported_at === null ? "从未（无报告）" : formatDateTime(report.reported_at)}</dd></div>
        </dl> : null}
        {report?.reported_at !== null && report?.state === "observed" && (!report.current || report.stale) ? <p role="status" className="text-warning">此报告已不属于当前配置 / 运行代际，或租约已失效；地址仅为历史观测。</p> : null}
        {!report || report.sockets.length === 0 ? <div className="flex flex-col gap-2 text-muted-foreground">
          <p>{report?.state === "withdrawn" || report?.state === "expired" ? "观测已撤回或过期，Controller 已清除 socket 地址。" : report?.state === "disabled" ? "此运行代际未启用 STUN 观测。" : report?.state === "probing" ? "正在等待实际 carrier socket 探测结果。" : "尚无 socket 报告；generation、绑定地址和映射均未知。"}</p>
          <p>在此节点现有 <code>run</code> 启动参数中显式加入 <code className="break-all">--stun-server &lt;NUMERIC_IP&gt;:3478</code>（IPv6 使用 <code>[&lt;NUMERIC_IPV6&gt;]:3478</code>）；沿用原有 --state、--apply-connections / --apply-dataplane 和 --connection-profiles。重新应用现有启动配置，不另起第二个 daemon。</p>
        </div> : report.sockets.map((socket) => <SocketReport key={`${socket.resource_kind}:${socket.resource_id}:${socket.generation}:${socket.socket_id}`} socket={socket} report={report} now={clock} />)}
      </>}
      <p className="text-muted-foreground">来源 managed_udp_socket_stun；使用实际 carrier socket。与独立临时 socket 的 netcheck 报告、管理员手工 advertised endpoint 分开，不自动拨号或更改配置；映射地址不保证是公网地址。</p>
    </div>
  </Panel>
}

function SocketReport({ socket, report, now }: { socket: UdpSocketDiscovery; report: NodeEndpointDiscovery; now: number }) {
  const historical = !report.current || report.stale || report.state === "withdrawn" || report.state === "expired" ||
    (socket.observations.length > 0 && socket.observations.every((observation) => Date.parse(observation.expires_at) <= now))
  return <details className="min-w-0 rounded-lg border" open>
    <summary className="cursor-pointer break-all p-3 font-medium focus-visible:outline-2 focus-visible:outline-ring">
      {socket.resource_kind === "listener" ? "Listener" : "Outgoing candidate"} · {socket.resource_id} · generation {socket.generation} / socket {socket.socket_id}
    </summary>
    <div className="flex min-w-0 flex-col gap-2 px-3 pb-3">
      <Address label={historical ? "历史本地绑定 IP:port（失效）" : "本地绑定 IP:port"} value={socket.bound_address} disabled={historical} />
      <p className="break-all text-muted-foreground">source：{socket.source}</p>
      {socket.observations.length === 0 ? <p className="text-muted-foreground">此 socket 尚无 observer 结果。</p> : socket.observations.map((observation) => {
        const usable = observationUsable(report, observation, now)
        const expired = Date.parse(observation.expires_at) <= now
        return <div key={`${observation.observer}:${observation.observed_at}`} data-testid="discovery-observation" className="flex min-w-0 flex-col gap-1.5 border-t pt-2">
          <div className="flex flex-wrap gap-2"><Badge variant="outline">{usable ? "未验证候选观测" : expired ? "已过期 · 不可使用" : discoveryOutcomeLabels[observation.outcome]}</Badge>{!usable && observation.mapped_endpoint !== null && !expired ? <span className="text-warning">已失效 / 非当前观测</span> : null}</div>
          {observation.mapped_endpoint === null ? <p>观测端点：无映射结果</p> : <Address label={usable ? "观测 IP:port" : "历史观测 IP:port（失效）"} value={observation.mapped_endpoint} disabled={!usable} />}
          <Address label="STUN observer" value={observation.observer} />
          <p className="text-muted-foreground">STUN RTT：{observation.rtt_ms === null ? "未测得" : `${observation.rtt_ms} ms`}（不是 peer RTT）</p>
          <p className="break-all text-muted-foreground">observed_at：{formatDateTime(observation.observed_at)} · expires_at：{formatDateTime(observation.expires_at)}</p>
        </div>
      })}
    </div>
  </details>
}

function Address({ label, value, disabled = false }: { label: string; value: string; disabled?: boolean }) {
  return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-start gap-2"><div className="min-w-0"><span className="text-muted-foreground">{label}：</span><code className="break-all">{value}</code></div><CopyButton value={value} label={`复制${label}`} iconOnly disabled={disabled} /></div>
}
