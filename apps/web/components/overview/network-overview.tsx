"use client"

import type { ConnectionsSnapshot, NodeSummary } from "@meshway/control-client"
import {
  ArrowRightIcon,
  CircleAlertIcon,
  LayoutPanelLeftIcon,
  MaximizeIcon,
  PlusIcon,
  RefreshCwIcon,
  XIcon,
} from "lucide-react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useEffect, useMemo, useRef, useState, useTransition, type ReactNode } from "react"

import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { StatusDot, type Tone } from "@/components/common/status-badge"
import { useConnectionRuntimes, type InitialRuntimes } from "@/components/connections/use-runtimes"
import { LastContact, ReceiptBadge } from "@/components/nodes/node-status"
import { BrandMark } from "@/components/shell/brand"
import { useConsoleFocus } from "@/components/shell/console-focus"
import { useNetwork } from "@/components/shell/network-context"
import { WorldDots } from "@/components/overview/world-dots"
import { Button, buttonVariants } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useSidebar } from "@/components/ui/sidebar"
import { diagnosticsHref } from "@/lib/connectivity-guidance"
import { transportLabels } from "@/lib/connections-model"
import { topologyFlowHref } from "@/lib/flow-navigation"
import type { Attention } from "@/lib/overview-attention"
import {
  edgeFacts,
  edgeLookLabels,
  nodePlacement,
  observeOverview,
  overviewSummary,
  relationshipLayout,
  runtimeRead,
  type EdgeFacts,
  type EdgeLook,
  type NodePlacement,
  type OverviewSummary,
  type RuntimeRead,
} from "@/lib/overview-map-model"
import { formatClock } from "@/lib/time"
import { reportUnusableLabels, summarizeNode, type TopologyObservation } from "@/lib/topology-model"
import { cn } from "@/lib/utils"

type Selection = { kind: "node"; id: string } | { kind: "edge"; key: string } | null

const LENS = 400

/**
 * The overview: a dotted world map with the network on it. With no
 * location source yet, nodes sit in a separate, labelled relationship lens
 * instead of on the map; links are only configured channels drawn by their
 * agents' own fresh reports.
 */
export function NetworkOverview({
  networkName,
  ipv4Prefix,
  nodes,
  fetchedAt,
  connections,
  initialRuntimes,
  attention,
}: {
  networkName: string
  ipv4Prefix: string | null
  nodes: NodeSummary[]
  fetchedAt: number
  connections: { ok: true; data: ConnectionsSnapshot } | { ok: false; error: string }
  initialRuntimes: InitialRuntimes
  attention: Attention[]
}) {
  const { routes } = useNetwork()
  const { focus, setFocus } = useConsoleFocus()
  const { isMobile, setOpenMobile } = useSidebar()
  const router = useRouter()
  const [reloading, startReload] = useTransition()
  const runtimes = useConnectionRuntimes(initialRuntimes)
  const [selection, setSelection] = useState<Selection>(null)

  const active = useMemo(() => nodes.filter((node) => node.revoked_at === null), [nodes])
  const placements = useMemo(() => new Map<string, NodePlacement>(active.map((node) => [node.id, nodePlacement(node)])), [active])
  const unplaced = active.filter((node) => placements.get(node.id)?.kind !== "geo")
  const observation = observeOverview(connections.ok ? connections.data : null, runtimes.snapshot, active, runtimes.now)
  const read = runtimeRead(runtimes)
  const summary = overviewSummary({ nodes: active, placements, observation, runtimes: read })
  const edges = observation ? observation.adjacencies.map(edgeFacts) : []
  const layout = useMemo(() => relationshipLayout(unplaced, LENS), [unplaced])
  const byId = new Map(active.map((node) => [node.id, node]))
  const selectedNode = selection?.kind === "node" ? byId.get(selection.id) : undefined
  const selectedEdge = selection?.kind === "edge" ? edges.find((edge) => edge.key === selection.key) : undefined

  // The mode button is replaced on switch; keep keyboard focus on its successor.
  const modeButton = useRef<HTMLButtonElement>(null)
  const moveFocus = useRef(false)
  const previousFocus = useRef(focus)
  useEffect(() => {
    if (previousFocus.current === focus) return
    previousFocus.current = focus
    // Also after ⌘B: the focused 管理网络 button no longer exists.
    if (moveFocus.current || document.activeElement === document.body) modeButton.current?.focus()
    moveFocus.current = false
  }, [focus])
  const manage = () => {
    setFocus(false)
    if (isMobile) setOpenMobile(true)
    else moveFocus.current = true
  }
  const refresh = () => {
    runtimes.refresh()
    startReload(() => router.refresh())
  }

  return (
    <section
      data-testid="network-overview"
      data-immersive={focus ? "true" : undefined}
      aria-label="网络概览"
      className="overview-stage relative isolate overflow-hidden"
      onKeyDown={(event) => {
        if (event.key === "Escape" && selection) setSelection(null)
      }}
    >
      <div aria-hidden="true" className="overview-backdrop absolute inset-0 -z-10" />
      <WorldDots />

      {/* Relationship lens: unplaced nodes, explicitly not geographic. */}
      <div className="absolute inset-0 flex items-center justify-center p-6 pb-24 sm:p-10 sm:pb-24">
        <div
          data-testid="overview-lens"
          className="relative aspect-square w-full max-w-[min(34rem,62svh)] rounded-full border border-dashed border-foreground/15 bg-background/55 shadow-[0_0_0_1px_color-mix(in_oklch,var(--foreground)_4%,transparent),0_30px_80px_-40px_color-mix(in_oklch,var(--brand)_45%,transparent)] backdrop-blur-[3px]"
        >
          <span className="absolute -top-3 left-1/2 -translate-x-1/2 rounded-full border bg-background/90 px-2.5 py-0.5 text-[0.6875rem] whitespace-nowrap text-muted-foreground shadow-xs">
            {unplaced.length > 0 ? `${unplaced.length} 个节点待定位 · 按连接关系排布，不代表地理位置` : "没有待定位的节点"}
          </span>
          {active.length === 0 ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 text-center">
              <p className="text-sm text-muted-foreground">这张网络还没有节点</p>
              <Link href={routes.addMachine} className={buttonVariants({ size: "sm" })}>
                <PlusIcon data-icon="inline-start" />
                添加节点
              </Link>
            </div>
          ) : (
            <LensGraph
              nodes={unplaced}
              layout={layout}
              edges={edges}
              observation={observation}
              selection={selection}
              onSelect={setSelection}
            />
          )}
        </div>
      </div>

      {focus ? (
        <div className="absolute top-4 left-4 flex items-center gap-2.5 rounded-xl border bg-background/75 py-1.5 pr-3 pl-1.5 shadow-sm backdrop-blur-md sm:top-6 sm:left-6">
          <BrandMark className="size-8" />
          <span className="grid text-left leading-tight">
            <span className="text-sm font-medium">{networkName}</span>
            {ipv4Prefix ? <span className="font-mono text-xs text-muted-foreground">{ipv4Prefix}</span> : null}
          </span>
        </div>
      ) : null}

      <div className="absolute top-4 right-4 flex items-center gap-2 sm:top-6 sm:right-6">
        {focus ? (
          <Button ref={modeButton} size="lg" onClick={manage} className="shadow-md">
            <LayoutPanelLeftIcon data-icon="inline-start" />
            管理网络
          </Button>
        ) : (
          <Button
            ref={modeButton}
            variant="outline"
            size="sm"
            onClick={() => {
              moveFocus.current = true
              setFocus(true)
            }}
            className="bg-background/80 backdrop-blur-md"
          >
            <MaximizeIcon data-icon="inline-start" />
            专注地图
          </Button>
        )}
      </div>

      {selectedNode ? (
        <FloatingCard onClose={() => setSelection(null)} label={`节点 ${selectedNode.name}`}>
          <NodeSummaryCard node={selectedNode} fetchedAt={fetchedAt} observation={observation} placement={placements.get(selectedNode.id)} />
        </FloatingCard>
      ) : selectedEdge ? (
        <FloatingCard onClose={() => setSelection(null)} label="连接">
          <EdgeCard edge={selectedEdge} byId={byId} read={read} />
        </FloatingCard>
      ) : null}

      <StatusBar
        summary={summary}
        connectionsError={connections.ok ? null : connections.error}
        read={read}
        attention={attention}
        updatedAt={runtimes.fetchedAt}
        refreshing={runtimes.refreshing || reloading}
        onRefresh={refresh}
      />
      <Legend />
    </section>
  )
}

const edgeStroke: Record<EdgeLook, { className: string; dash?: string; width: number }> = {
  confirmed: { className: "text-success", width: 2.4 },
  one_sided: { className: "text-warning", width: 2 },
  degraded: { className: "text-destructive", width: 1.6, dash: "5 4" },
  configured: { className: "text-muted-foreground/70", width: 1.4, dash: "1.5 5" },
}

function LensGraph({
  nodes,
  layout,
  edges,
  observation,
  selection,
  onSelect,
}: {
  nodes: NodeSummary[]
  layout: Map<string, { x: number; y: number }>
  edges: EdgeFacts[]
  observation: TopologyObservation | null
  selection: Selection
  onSelect: (selection: Selection) => void
}) {
  const names = new Map(nodes.map((node) => [node.id, node.name]))
  const select = (next: Selection) => onSelect(selection && next && JSON.stringify(selection) === JSON.stringify(next) ? null : next)
  return (
    <svg viewBox={`0 0 ${LENS} ${LENS}`} className="absolute inset-0 size-full overflow-visible" role="group" aria-label="节点与连接">
      {edges.map((edge) => {
        const a = layout.get(edge.nodeIds[0])
        const b = layout.get(edge.nodeIds[1])
        if (!a || !b) return null
        const stroke = edgeStroke[edge.look]
        const selected = selection?.kind === "edge" && selection.key === edge.key
        const label = `${names.get(edge.nodeIds[0]) ?? "?"} 与 ${names.get(edge.nodeIds[1]) ?? "?"}：${edgeLookLabels[edge.look]}`
        return (
          <g
            key={edge.key}
            role="button"
            tabIndex={0}
            aria-label={label}
            aria-pressed={selected}
            data-testid="overview-edge"
            data-look={edge.look}
            className={cn("cursor-pointer outline-none [&:focus-visible_.edge-focus]:opacity-100", stroke.className)}
            onClick={() => select({ kind: "edge", key: edge.key })}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault()
                select({ kind: "edge", key: edge.key })
              }
            }}
          >
            <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} stroke="transparent" strokeWidth={16} />
            <line
              className="edge-focus opacity-0 transition-opacity"
              x1={a.x} y1={a.y} x2={b.x} y2={b.y}
              stroke="var(--ring)" strokeOpacity={0.35} strokeWidth={8} strokeLinecap="round"
              style={selected ? { opacity: 1 } : undefined}
            />
            <line
              x1={a.x} y1={a.y} x2={b.x} y2={b.y}
              stroke="currentColor" strokeWidth={selected ? stroke.width + 1 : stroke.width} strokeDasharray={stroke.dash} strokeLinecap="round"
            />
          </g>
        )
      })}
      {nodes.map((node) => {
        const point = layout.get(node.id)
        if (!point) return null
        const report = observation?.reports.get(node.id)
        const reporting = report?.usable === true
        const selected = selection?.kind === "node" && selection.id === node.id
        const summary = observation ? summarizeNode(observation, node.id) : null
        const below = point.y >= LENS / 2 - 1
        return (
          <g
            key={node.id}
            role="button"
            tabIndex={0}
            aria-label={`${node.name}，${reporting ? "有最新运行报告" : "无最新运行报告"}${summary ? `，${summary.confirmed + summary.oneSided} / ${summary.configured} 条连接有认证证据` : ""}`}
            aria-pressed={selected}
            data-testid="overview-node"
            className="cursor-pointer outline-none [&:focus-visible_.node-focus]:opacity-100"
            onClick={() => select({ kind: "node", id: node.id })}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault()
                select({ kind: "node", id: node.id })
              }
            }}
          >
            <circle className="node-focus opacity-0 transition-opacity" cx={point.x} cy={point.y} r={20} fill="var(--ring)" fillOpacity={0.18} style={selected ? { opacity: 1 } : undefined} />
            <circle cx={point.x} cy={point.y} r={11} className={cn(reporting ? "fill-brand/15 stroke-brand" : "fill-card stroke-muted-foreground/50")} strokeWidth={selected ? 2.5 : 1.6} />
            <circle cx={point.x} cy={point.y} r={4} className={reporting ? "fill-brand" : "fill-muted-foreground/60"} />
            <text
              x={point.x}
              y={below ? point.y + 28 : point.y - 31}
              textAnchor="middle"
              className="fill-foreground text-[11px] font-medium [paint-order:stroke] [stroke:var(--background)] [stroke-width:3px]"
            >
              {node.name}
            </text>
            <text
              x={point.x}
              y={below ? point.y + 41 : point.y - 18}
              textAnchor="middle"
              className="fill-muted-foreground font-mono text-[9px] [paint-order:stroke] [stroke:var(--background)] [stroke-width:3px]"
            >
              {node.mesh_ipv4}
            </text>
          </g>
        )
      })}
    </svg>
  )
}

function FloatingCard({ label, onClose, children }: { label: string; onClose: () => void; children: ReactNode }) {
  return (
    <aside
      aria-label={label}
      data-testid="overview-selection"
      className="absolute inset-x-3 bottom-20 z-10 max-h-[55svh] overflow-auto rounded-2xl border bg-popover/95 p-4 shadow-xl backdrop-blur-md sm:inset-x-auto sm:top-20 sm:right-6 sm:bottom-auto sm:w-[22rem] sm:max-h-[calc(100%-10rem)]"
    >
      <Button variant="ghost" size="icon-sm" className="absolute top-2.5 right-2.5" aria-label="关闭" onClick={onClose}>
        <XIcon />
      </Button>
      {children}
    </aside>
  )
}

function NodeSummaryCard({
  node,
  fetchedAt,
  observation,
  placement,
}: {
  node: NodeSummary
  fetchedAt: number
  observation: TopologyObservation | null
  placement: NodePlacement | undefined
}) {
  const { routes } = useNetwork()
  const report = observation?.reports.get(node.id)
  const counts = observation ? summarizeNode(observation, node.id) : null
  return (
    <div className="flex flex-col gap-3 text-[0.8125rem]">
      <div className="pr-8">
        <h2 className="text-base font-semibold tracking-tight">{node.name}</h2>
        <p className="font-mono text-xs text-muted-foreground">{node.mesh_ipv4}</p>
      </div>
      <dl className="grid grid-cols-[5.5rem_minmax(0,1fr)] gap-x-3 gap-y-2 text-xs">
        <dt className="text-muted-foreground">控制面联系</dt>
        <dd><LastContact node={node} fetchedAt={fetchedAt} /></dd>
        <dt className="text-muted-foreground">配置确认</dt>
        <dd><ReceiptBadge node={node} /></dd>
        <dt className="text-muted-foreground">运行报告</dt>
        <dd>{!report ? "未知" : report.usable ? "最新" : reportUnusableLabels[report.reason]}</dd>
        <dt className="text-muted-foreground">连接</dt>
        <dd>{counts ? (counts.configured === 0 ? "没有已配置的连接" : `${counts.confirmed} 双向确认 · ${counts.oneSided} 单侧 / ${counts.configured} 已配置`) : "连接配置不可用"}</dd>
        <dt className="text-muted-foreground">位置</dt>
        <dd>{placement?.kind === "geo" ? `${placement.source} · ${formatClock(Date.parse(placement.observedAt))}` : "待定位（尚无位置来源）"}</dd>
      </dl>
      <div className="flex flex-wrap gap-2">
        <Link href={routes.machine(node.id, "connections")} className={buttonVariants({ size: "sm" })}>
          节点配置
          <ArrowRightIcon data-icon="inline-end" />
        </Link>
        <Link href={routes.machine(node.id)} className={buttonVariants({ variant: "outline", size: "sm" })}>详情</Link>
        <Link href={diagnosticsHref(routes, node.id)} className={buttonVariants({ variant: "ghost", size: "sm" })}>诊断</Link>
      </div>
    </div>
  )
}

function EdgeCard({ edge, byId, read }: { edge: EdgeFacts; byId: Map<string, NodeSummary>; read: RuntimeRead }) {
  const { routes } = useNetwork()
  const name = (id: string) => byId.get(id)?.name ?? "未知节点"
  const [a, b] = edge.nodeIds
  const tone: Tone = edge.look === "confirmed" ? "success" : edge.look === "one_sided" ? "warning" : edge.look === "degraded" ? "danger" : "neutral"
  return (
    <div className="flex flex-col gap-3 text-[0.8125rem]">
      <div className="pr-8">
        <h2 className="text-base font-semibold tracking-tight">{name(a)} ⇄ {name(b)}</h2>
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <StatusDot tone={tone} />
          {edgeLookLabels[edge.look]}
        </p>
      </div>
      {read.kind === "cached" ? (
        <p role="status" className="rounded-lg border border-warning/40 bg-warning/10 px-2.5 py-1.5 text-xs">
          运行报告刷新失败，以下是 {formatClock(read.at)} 读取的报告（仍按原有效期过期）。
        </p>
      ) : null}
      <ul className="flex flex-col gap-2.5">
        {edge.channels.map((channel) => (
          <li key={channel.candidateId} data-testid="overview-channel" className="rounded-xl border bg-background/60 p-3">
            <p className="flex items-center justify-between gap-2 text-xs">
              <span className="font-medium">{transportLabels[channel.transport]}</span>
              <span className="text-muted-foreground">{name(channel.dialerNodeId)} 发起</span>
            </p>
            <dl className="mt-2 grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 text-xs">
              <dt className="truncate text-muted-foreground">{name(channel.dialerNodeId)}：{channel.dialer.label}</dt>
              <dd className="font-mono tabular-nums">{channel.dialer.rtt ?? "—"}</dd>
              <dt className="truncate text-muted-foreground">{name(channel.acceptorNodeId)}：{channel.acceptor.label}</dt>
              <dd className="font-mono tabular-nums">{channel.acceptor.rtt ?? "—"}</dd>
            </dl>
          </li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">RTT 是各端认证探测的往返时间，分别显示。</p>
      <section aria-label="延迟历史" className="flex flex-col gap-1.5">
        <h3 className="text-xs font-medium">延迟历史</h3>
        <div data-testid="overview-history-empty" className="flex h-12 items-center justify-center rounded-lg border border-dashed text-xs text-muted-foreground">
          尚无历史数据
        </div>
      </section>
      <div className="flex flex-wrap gap-2">
        <Link href={topologyFlowHref(routes, { sourceNodeId: a, targetNodeId: b })} className={buttonVariants({ variant: "outline", size: "sm" })}>
          检查访问与路径
        </Link>
        <Link href={routes.topology({ node: a })} className={buttonVariants({ variant: "ghost", size: "sm" })}>拓扑</Link>
      </div>
      <DiagnosticDetails title="技术详情">
        <ul className="flex flex-col gap-1 font-mono">
          {edge.channels.map((channel) => (
            <li key={channel.candidateId}>candidate {channel.candidateId} · listener {channel.listenerId} · {channel.status}</li>
          ))}
        </ul>
      </DiagnosticDetails>
    </div>
  )
}

function StatusBar({
  summary,
  connectionsError,
  read,
  attention,
  updatedAt,
  refreshing,
  onRefresh,
}: {
  summary: OverviewSummary
  connectionsError: string | null
  read: RuntimeRead
  attention: Attention[]
  updatedAt: number | null
  refreshing: boolean
  onRefresh: () => void
}) {
  const connections = summary.connections
  const linkText =
    connections.kind === "unknown"
      ? connections.reason === "plan"
        ? "连接未知：配置读取失败"
        : "连接未知：运行报告读取失败"
      : connections.kind === "none"
        ? "没有已配置的连接"
        : `${connections.cachedAt !== null ? `缓存 ${formatClock(connections.cachedAt)}：` : ""}${connections.confirmed} / ${connections.configured} 连接双向确认${connections.oneSided ? ` · ${connections.oneSided} 单侧` : ""}${connections.unverified ? ` · ${connections.unverified} 未验证` : ""}`
  const linkTone: Tone =
    connections.kind === "unknown"
      ? "danger"
      : connections.kind === "none"
        ? "neutral"
        : connections.cachedAt !== null || connections.unverified + connections.oneSided > 0
          ? "warning"
          : "success"
  const failure = connectionsError !== null
    ? `连接配置读取失败：${connectionsError}`
    : read.kind === "failed"
      ? `运行报告读取失败：${read.error}`
      : read.kind === "cached"
        ? `运行报告刷新失败：${read.error}`
        : null
  return (
    <div
      data-testid="overview-status"
      className="absolute bottom-3 left-3 flex max-w-[calc(100%-1.5rem)] flex-wrap lg:max-w-[min(44rem,calc(100%-15rem))] items-center gap-x-4 gap-y-1.5 rounded-2xl border bg-background/80 px-4 py-2.5 text-xs shadow-sm backdrop-blur-md sm:bottom-6 sm:left-6"
    >
      <span className="flex items-center gap-1.5">
        <span className="text-lg leading-none font-semibold tabular-nums">{summary.nodes}</span>
        <span className="text-muted-foreground">节点</span>
      </span>
      <span className="flex items-center gap-1.5">
        <StatusDot tone={linkTone} />
        {linkText}
      </span>
      {attention.length > 0 ? (
        <Popover>
          <PopoverTrigger className="flex items-center gap-1.5 rounded-md text-warning outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/40">
            <CircleAlertIcon className="size-3.5" aria-hidden="true" />
            {attention.length} 项待处理
          </PopoverTrigger>
          <PopoverContent side="top" align="start" className="w-80 gap-1 p-1.5">
            {attention.map((item) => (
              <Link key={item.kind} href={item.href} className="flex flex-col gap-0.5 rounded-md px-2.5 py-2 hover:bg-accent">
                <span className={cn("text-[0.8125rem] font-medium", item.tone === "danger" && "text-destructive")}>{item.title}</span>
                <span className="text-xs text-muted-foreground">{item.detail}</span>
              </Link>
            ))}
          </PopoverContent>
        </Popover>
      ) : null}
      <span className="flex items-center gap-1 text-muted-foreground tabular-nums">
        {updatedAt === null ? "尚无成功的运行报告读取" : read.kind === "current" ? `更新于 ${formatClock(updatedAt)}` : `上次成功 ${formatClock(updatedAt)}`}
        <Button variant="ghost" size="icon-sm" aria-label="刷新概览" disabled={refreshing} onClick={onRefresh}>
          <RefreshCwIcon className={refreshing ? "animate-spin" : undefined} />
        </Button>
      </span>
      {failure ? (
        <p role="alert" data-testid="overview-read-error" className="flex w-full min-w-0 items-center gap-2 text-destructive">
          <CircleAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
          <span className="min-w-0 flex-1 break-words">{failure}</span>
          <Button variant="outline" size="sm" disabled={refreshing} onClick={onRefresh}>
            重试
          </Button>
        </p>
      ) : null}
    </div>
  )
}

function Legend() {
  const items: { look: EdgeLook; label: string }[] = [
    { look: "confirmed", label: "双向确认" },
    { look: "one_sided", label: "单侧报告" },
    { look: "degraded", label: "未认证" },
    { look: "configured", label: "仅配置" },
  ]
  return (
    <ul aria-label="连线图例" className="absolute right-6 bottom-6 hidden flex-col gap-1 rounded-xl border bg-background/70 px-3 py-2 text-[0.6875rem] text-muted-foreground backdrop-blur-md lg:flex">
      {items.map((item) => (
        <li key={item.look} className="flex items-center gap-2">
          <svg viewBox="0 0 24 4" className={cn("h-1 w-6", edgeStroke[item.look].className)} aria-hidden="true">
            <line x1="1" y1="2" x2="23" y2="2" stroke="currentColor" strokeWidth={2} strokeDasharray={edgeStroke[item.look].dash} strokeLinecap="round" />
          </svg>
          {item.label}
        </li>
      ))}
    </ul>
  )
}
