"use client"

import { useNetworkId } from "@/components/network/network-workspace"

import type { ConnectionNode, ConnectionsSnapshot, EndpointDiscoveriesSnapshot, NodeSummary } from "@meshway/control-client"
import {
  ArrowLeftIcon,
  CircleAlertIcon,
  RefreshCwIcon,
  CableIcon,
  FlagIcon,
  ShieldOffIcon,
  ShieldCheckIcon,
  WaypointsIcon,
} from "lucide-react"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useCallback, useState, type ReactNode } from "react"

import { ManagedDataplanePanel } from "@/components/nodes/managed-dataplane-panel"
import { NodeDnsPanel, type NodeDnsNetwork } from "@/components/nodes/node-dns-panel"
import { ConnectionsEditor } from "@/components/connections/connections-editor"
import { useConnectionsHasDraft } from "@/components/connections/draft-store"
import { ConnectionIdentityDetails, ConnectionReceipt } from "@/components/connections/node-connection-facts"
import { EndpointDiscoveryPanel } from "@/components/connections/endpoint-discovery-panel"
import { NodeRuntimePanel, RuntimeChip } from "@/components/connections/runtime-panel"
import { useConnectionRuntimes, type InitialRuntimes, type Runtimes } from "@/components/connections/use-runtimes"
import { ControllerUnavailable } from "@/components/common/controller-states"
import { CopyButton } from "@/components/common/copy-button"
import { Panel } from "@/components/common/panel"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { SectionHeader } from "@/components/common/section-header"
import { Metric, StatusSummary } from "@/components/common/status-summary"
import { FailureNotice } from "@/components/common/failure-notice"
import { CountryDialog } from "@/components/nodes/country-dialog"
import { LastContact, ReceiptBadge, StatusBadge } from "@/components/nodes/node-status"
import { RevokeDialog } from "@/components/nodes/revoke-dialog"
import { Badge } from "@/components/ui/badge"
import { Button, buttonVariants } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { runtimeFor } from "@/lib/connection-runtime-model"
import {
  fromPlan,
  latestSnapshot,
  nodeScope,
  readiness,
  readinessLabels,
  type StampedSnapshot,
} from "@/lib/connections-model"
import type { RuntimeReadResult } from "@/lib/action-state"
import type { ControllerResult } from "@/lib/controller"
import { readNodeStatusAction } from "@/lib/connections-actions"
import { nextPartStatus, type PartStatus } from "@/lib/node-status"
import { formatClock, formatDateTime } from "@/lib/time"
import { topologyFlowHref } from "@/lib/flow-navigation"
import { diagnosticsHref } from "@/lib/connectivity-guidance"

export type NodeTab = "overview" | "connections"

type ConnectionsRead = { ok: true; data: ConnectionsSnapshot } | { ok: false; error: string }
/** Page render input: the read also carries the server time it was made at. */
type ConnectionsPageRead = { ok: true; data: ConnectionsSnapshot; fetchedAt: number } | { ok: false; error: string }

/**
 * One node: identity and control-plane contact on the overview tab; its
 * saved connection intent, receipts and self-reported runtime on the
 * connections tab. Those three are deliberately separate surfaces.
 */
export function NodeDetail({
  node: renderedNode,
  fetchedAt: renderedAt,
  initialTab,
  connections,
  controllerUrl,
  initialRuntimes,
  discoveries,
  tags,
  network,
}: {
  node: NodeSummary
  fetchedAt: number
  initialTab: NodeTab
  connections: ConnectionsPageRead
  controllerUrl: string
  initialRuntimes: InitialRuntimes
  discoveries: ControllerResult<EndpointDiscoveriesSnapshot>
  /** This node's tags, or an error when the tag snapshot could not be read. */
  tags: { ok: true; tags: string[] } | { ok: false; error: string }
  network: NodeDnsNetwork
}) {
  // The URL is the source of truth: history navigation can restore a cached
  // render whose server-provided tab is older than the entry's URL.
  const params = useSearchParams()
  const networkId = useNetworkId()
  const hasConnectionDraft = useConnectionsHasDraft()
  const tab: NodeTab = params.has("tab") ? (params.get("tab") === "connections" ? "connections" : "overview") : initialTab
  const [dialog, setDialog] = useState<"country" | "revoke" | null>(null)
  // Snapshot sources: the page render, the status poll, and action responses
  // (successful save, explicit reload). The action response outlives the
  // editor, which the tabs unmount, so remounting never resurrects an older
  // plan; a later read at the same revision still brings newer receipts.
  // A kept dirty draft restores its own CAS base from the draft store.
  const [adopted, setAdopted] = useState<StampedSnapshot | null>(null)
  const [polled, setPolled] = useState<StampedSnapshot | null>(null)
  const [polledNode, setPolledNode] = useState<{ node: NodeSummary; fetchedAt: number } | null>(null)
  const chosen = latestSnapshot(
    connections.ok ? { snapshot: connections.data, readAt: connections.fetchedAt } : null,
    polled,
    adopted
  )
  const latest: ConnectionsRead = chosen ? { ok: true, data: chosen.snapshot } : connections
  const shown = polledNode !== null && polledNode.fetchedAt > renderedAt ? polledNode : { node: renderedNode, fetchedAt: renderedAt }
  const node = shown.node
  const fetchedAt = shown.fetchedAt

  // One bounded poll (via the runtime hook's interval) refreshes contact,
  // IP / connection receipts and runtime reports together. Results are
  // display sources only: they never move an editor's base or forms.
  const nodeId = renderedNode.id
  // Background read outcome per source; last-good values stay on screen.
  const [inventoryRead, setInventoryRead] = useState<PartStatus>({ at: renderedAt, error: null })
  const [connectionsRead, setConnectionsRead] = useState<PartStatus>({
    at: connections.ok ? connections.fetchedAt : 0,
    error: connections.ok ? null : connections.error,
  })
  const poll = useCallback(async (): Promise<RuntimeReadResult> => {
    const result = await readNodeStatusAction(networkId)
    if (result.status === "error") return result
    const read = result.fetchedAt
    const found = result.nodes.ok ? result.nodes.data.nodes.find((candidate) => candidate.id === nodeId) : undefined
    if (found) setPolledNode((current) => (current && current.fetchedAt >= read ? current : { node: found, fetchedAt: read }))
    const inventoryError = !result.nodes.ok ? result.nodes.error : found ? null : "节点不在清单中"
    setInventoryRead((current) => nextPartStatus(current, read, inventoryError))
    if (result.connections.ok) {
      const stamped = { snapshot: result.connections.data, readAt: read }
      setPolled((current) => latestSnapshot(current, stamped))
    }
    setConnectionsRead((current) => nextPartStatus(current, read, result.connections.ok ? null : result.connections.error))
    return result.runtimes.ok
      ? { status: "loaded", snapshot: result.runtimes.data, fetchedAt: read }
      : { status: "error", message: result.runtimes.error }
  }, [nodeId, networkId])
  const runtimes = useConnectionRuntimes(initialRuntimes, poll)
  const revoked = node.revoked_at !== null
  const entry = latest.ok ? latest.data.nodes.find((candidate) => candidate.node.id === node.id) : undefined
  const savedScope = latest.ok ? nodeScope(fromPlan(latest.data.configuration), node.id) : null

  const changeTab = (next: NodeTab) => {
    const url = new URL(window.location.href)
    url.searchParams.set("tab", next)
    window.history.replaceState(null, "", url)
  }

  return (
    <div className="flex flex-col gap-6">
      <header className="flex flex-col gap-3">
        <Link
          href="/nodes"
          className="inline-flex w-fit items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        >
          <ArrowLeftIcon className="size-3.5" />
          全部节点
        </Link>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
          <div className="flex min-w-0 flex-col gap-1">
            <div className="flex min-w-0 items-center gap-2">
              <h1 className="truncate text-xl font-semibold tracking-tight">{node.name}</h1>
              <StatusBadge node={node} />
            </div>
            <p className="text-xs text-muted-foreground">{node.mesh_ipv4} · <LastContact node={node} fetchedAt={fetchedAt} /></p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Link
              href={`/topology?node=${encodeURIComponent(node.id)}`}
              className={buttonVariants({ variant: "outline" })}
            >
              <WaypointsIcon data-icon="inline-start" />
              在拓扑中查看
            </Link>
            {!revoked ? (
              <Link href={topologyFlowHref({ sourceNodeId: node.id })} className={buttonVariants({ variant: "outline" })}>
                <ShieldCheckIcon data-icon="inline-start" />
                检查访问
              </Link>
            ) : null}
            {!revoked ? (
              <>
                <Button variant="outline" onClick={() => setDialog("country")}>
                  <FlagIcon data-icon="inline-start" />
                  设置国家
                </Button>
                <Button variant="outline" className="text-destructive" onClick={() => setDialog("revoke")}>
                  <ShieldOffIcon data-icon="inline-start" />
                  撤销…
                </Button>
              </>
            ) : null}
          </div>
        </div>
      </header>

      <StaleReadHint
        failures={[
          // Stale only while no newer source (render, action response) replaced it.
          inventoryRead.error !== null && inventoryRead.at > fetchedAt
            ? { label: "节点状态", error: inventoryRead.error, lastGood: fetchedAt }
            : null,
          connectionsRead.error !== null && chosen !== null && connectionsRead.at > chosen.readAt
            ? { label: "连接配置与回执", error: connectionsRead.error, lastGood: chosen.readAt }
            : null,
        ]}
        refreshing={runtimes.refreshing}
        onRetry={runtimes.refresh}
      />

      <Tabs value={tab} onValueChange={(value) => changeTab(value === "connections" ? "connections" : "overview")}>
        <TabsList variant="line" className="w-full justify-start border-b">
          <TabsTrigger value="overview" className="flex-none px-3">
            概览
          </TabsTrigger>
          <TabsTrigger value="connections" className="flex-none px-3">
            <CableIcon />
            连接
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="pt-4">
          <Overview
            node={node}
            fetchedAt={fetchedAt}
            entry={entry}
            connections={latest}
            runtimes={runtimes}
            onOpenConnections={() => changeTab("connections")}
            onSetCountry={revoked ? null : () => setDialog("country")}
            tags={tags}
            network={network}
          />
        </TabsContent>

        <TabsContent value="connections" className="flex flex-col gap-4 pt-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">运行观测来自 Agent 上报；本地检查可在诊断工作台生成命令。</p>
            <Link href={diagnosticsHref(node.id)} className={buttonVariants({ variant: "outline", size: "sm" })}>诊断此节点</Link>
          </div>
          {latest.ok ? (
            <>
              <StatusSummary>
                <Metric label="已保存意图">{savedScope?.listeners.length ?? 0} 个监听 · {savedScope?.outgoing.length ?? 0} 个主动连接</Metric>
                {entry ? <Metric label="节点回执"><ConnectionReceipt entry={entry} revision={latest.data.revision} /></Metric> : null}
                {entry && readiness(entry) !== "ready" ? <span className="text-warning">{readinessLabels[readiness(entry)]} · 展开连接身份详情检查</span> : null}
              </StatusSummary>
              <NodeRuntimePanel nodeId={node.id} nodes={latest.data.nodes} runtimes={runtimes} />
              <DiagnosticDetails title="UDP 地址观测与当前启用帮助">
                <EndpointDiscoveryPanel initial={discoveries} nodeId={node.id} />
                {entry ? <ConnectionIdentityDetails entry={entry} revision={latest.data.revision} /> : null}
                {!revoked ? <ManagedDataplanePanel /> : null}
              </DiagnosticDetails>
              <DiagnosticDetails title="高级手工连接配置" open={hasConnectionDraft || !latest.data.valid}>
                <SectionHeader id="node-connections-plan" title="连接配置（草稿）" description="当前可用的监听与主动连接配置。保存意图；实际连接以上方运行报告为准。" />
                <ConnectionsEditor
                  snapshot={latest.data}
                  scopeNodeId={node.id}
                  runtimes={runtimes}
                  onSnapshot={(snapshot, readAt) =>
                    setAdopted((current) => latestSnapshot(current, { snapshot, readAt }))
                  }
                />
              </DiagnosticDetails>
            </>
          ) : (
            <ControllerUnavailable url={controllerUrl} error={latest.error} />
          )}
        </TabsContent>
      </Tabs>

      {!revoked ? (
        <>
          <CountryDialog node={node} open={dialog === "country"} onOpenChange={(open) => setDialog(open ? "country" : null)} />
          <RevokeDialog node={node} open={dialog === "revoke"} onOpenChange={(open) => setDialog(open ? "revoke" : null)} />
        </>
      ) : null}
    </div>
  )
}

function Overview({
  node,
  fetchedAt,
  entry,
  connections,
  runtimes,
  onOpenConnections,
  onSetCountry,
  tags,
  network,
}: {
  node: NodeSummary
  fetchedAt: number
  entry: ConnectionNode | undefined
  connections: ConnectionsRead
  runtimes: Runtimes
  onOpenConnections: () => void
  onSetCountry: (() => void) | null
  tags: { ok: true; tags: string[] } | { ok: false; error: string }
  network: NodeDnsNetwork
}) {
  const scope = connections.ok ? nodeScope(fromPlan(connections.data.configuration), node.id) : null
  return (
    <div className="grid items-start gap-4 lg:grid-cols-2">
      <Panel title="身份与地址" bodyClassName="px-4 py-1">
        <dl className="divide-y">
          <Row label="名称">{node.name}</Row>
          <Row label="Mesh IPv4" hint="隧道内地址，不是其他节点建连用的地址">
            <CopyValue value={node.mesh_ipv4} label="复制 IPv4" />
          </Row>
          <Row label="Mesh IPv6">
            <CopyValue value={node.mesh_ipv6} label="复制 IPv6" />
          </Row>
          <Row label="国家" hint="管理员标注，用于路由匹配，不是定位结果">
            <span className="flex items-center justify-between gap-2">
              <span className="font-mono">{node.country ?? <span className="font-sans text-muted-foreground">未设置</span>}</span>
              {onSetCountry ? (
                <Button variant="ghost" size="sm" onClick={onSetCountry}>
                  修改
                </Button>
              ) : null}
            </span>
          </Row>
          <Row label="标签" hint="管理员分组，用于访问规则">
            <span className="flex items-center justify-between gap-2">
              {!tags.ok ? (
                <span className="text-destructive">读取失败</span>
              ) : tags.tags.length === 0 ? (
                <span className="text-muted-foreground">没有标签</span>
              ) : (
                <span data-testid="node-tags" className="flex flex-wrap gap-1">
                  {tags.tags.map((tag) => (
                    <Badge key={tag} variant="outline" className="font-mono">
                      {tag.slice(4)}
                    </Badge>
                  ))}
                </span>
              )}
              {node.revoked_at === null ? (
                <Link
                  href={`/tags?node=${encodeURIComponent(node.id)}`}
                  className={buttonVariants({ variant: "ghost", size: "sm" })}
                >
                  修改
                </Link>
              ) : null}
            </span>
          </Row>
          {node.revoked_at !== null ? (
            <Row label="撤销于">
              <span className="font-mono">{formatDateTime(node.revoked_at)}</span>
            </Row>
          ) : null}
        </dl>
        <div className="py-3">
          <DiagnosticDetails title="节点身份技术详情">
            <dl>
              <Row label="节点 ID"><CopyValue value={node.id} label="复制节点 ID" /></Row>
              <Row label="Agent 版本"><span className="font-mono">{node.agent_version}</span></Row>
              <Row label="注册于">{formatDateTime(node.registered_at)}</Row>
            </dl>
          </DiagnosticDetails>
        </div>
      </Panel>

      <div className="flex flex-col gap-4">
        <NodeDnsPanel node={node} network={network} />
        <Panel title="控制面" bodyClassName="px-4 py-1">
          <dl className="divide-y">
            <Row label="最后联系" hint="controller 最近一次接受注册或心跳，不代表在线或连通">
              <LastContact node={node} fetchedAt={fetchedAt} detail />
            </Row>
            <Row label="IP / 路由配置">
              <ReceiptBadge node={node} />
            </Row>
          </dl>
        </Panel>

        <Panel
          title="连接"
          actions={
            <Button size="sm" variant="outline" onClick={onOpenConnections}>
              <CableIcon data-icon="inline-start" />
               查看连接
            </Button>
          }
          bodyClassName="px-4 py-1"
        >
          {!connections.ok ? (
            <div className="py-3"><FailureNotice title="连接配置读取失败" nextStep="进入连接页重试；持续失败时检查控制面。" error={connections.error} /></div>
          ) : !entry ? (
            <p className="py-3 text-xs text-muted-foreground">连接清单中没有这个节点。</p>
          ) : (
            <div>
            <dl className="divide-y">
              <Row label="已保存配置">
                {scope
                  ? `接收 ${scope.listeners.length} 个监听 · 主动连接 ${scope.outgoing.length} 个 · 被连接 ${scope.incoming.length} 个`
                  : "—"}
              </Row>
              <Row label="节点回执">
                <ConnectionReceipt entry={entry} revision={connections.data.revision} />
              </Row>
              <Row label="运行状态">
                <RuntimeChip
                  observation={runtimeFor(runtimes.snapshot, node.id)}
                  now={runtimes.now}
                  readFailed={runtimes.snapshot === null}
                />
              </Row>
            </dl>
            <div className="py-3"><ConnectionIdentityDetails entry={entry} revision={connections.data.revision} /></div>
            </div>
          )}
        </Panel>

        {node.revoked_at === null ? <DiagnosticDetails title="当前节点启用帮助"><ManagedDataplanePanel /></DiagnosticDetails> : null}
      </div>
    </div>
  )
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[6.5rem_minmax(0,1fr)] items-start gap-3 py-2.5 sm:grid-cols-[8rem_minmax(0,1fr)]">
      <dt className="flex flex-col gap-0.5 text-xs text-muted-foreground">
        {label}
        {hint ? <span className="text-[0.625rem] leading-snug text-muted-foreground/80">{hint}</span> : null}
      </dt>
      <dd className="min-w-0 text-xs">{children}</dd>
    </div>
  )
}

function CopyValue({ value, label }: { value: string; label: string }) {
  return (
    <span className="flex min-w-0 items-center justify-between gap-2">
      <code className="min-w-0 font-mono text-xs break-all">{value}</code>
      <CopyButton value={value} label={label} iconOnly variant="ghost" />
    </span>
  )
}

/**
 * One compact line when a background read failed: the values shown are the
 * last successful read, not current state. Runtime failures are reported by
 * the runtime panel itself.
 */
function StaleReadHint({
  failures,
  refreshing,
  onRetry,
}: {
  failures: ({ label: string; error: string; lastGood: number | null } | null)[]
  refreshing: boolean
  onRetry: () => void
}) {
  const failed = failures.filter((failure) => failure !== null)
  if (failed.length === 0) return null
  const lastGood = Math.min(...failed.map((failure) => failure.lastGood ?? Number.POSITIVE_INFINITY))
  return (
    <div
      role="status"
      data-testid="node-status-stale"
      className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border border-warning/40 bg-warning/5 px-3 py-2 text-xs"
    >
      <CircleAlertIcon className="size-3.5 shrink-0 text-warning" aria-hidden="true" />
      <span className="font-medium">
        {failed.map((failure) => failure.label).join("、")}读取失败，显示
        {Number.isFinite(lastGood) ? ` ${formatClock(lastGood)} ` : "上次"}的结果
      </span>
      <span className="min-w-0 flex-1 truncate text-muted-foreground" title={failed.map((failure) => failure.error).join("\n")}>
        {failed[0]?.error}
      </span>
      <Button variant="ghost" size="sm" disabled={refreshing} onClick={onRetry}>
        <RefreshCwIcon data-icon="inline-start" className={refreshing ? "animate-spin" : undefined} />
        重试
      </Button>
    </div>
  )
}
