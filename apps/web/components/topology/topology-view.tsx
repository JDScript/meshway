"use client"

import { useRoutes } from "@/components/shell/network-context"
import { useNetworkId } from "@/components/network/network-workspace"

import type { ConnectionsSnapshot, NodeSummary, PolicySnapshot } from "@meshway/control-client"
import { ArrowRightIcon, CircleAlertIcon, RefreshCwIcon } from "lucide-react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useCallback, useRef, useState, useTransition } from "react"

import { Panel } from "@/components/common/panel"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { useConnectionRuntimes, type InitialRuntimes } from "@/components/connections/use-runtimes"
import { LastContact, ReceiptBadge } from "@/components/nodes/node-status"
import { planSteps } from "@/components/routing/path-trace"
import {
  externalLabel,
  FlowForm,
  PreviewButton,
  PreviewOutcome,
} from "@/components/routing/route-preview"
import { AdjacencyBadge, AdjacencyList, ChannelRows, channelText, NodeLink } from "@/components/topology/adjacency-list"
import { FlowStages } from "@/components/topology/flow-stages"
import { MeshMap, type MapEdge, type MapPlan } from "@/components/topology/mesh-map"
import { Badge } from "@/components/ui/badge"
import { Button, buttonVariants } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { previewFlowAction } from "@/lib/access-actions"
import type { FlowPreviewResult, PreviewResult, RuntimeReadResult } from "@/lib/action-state"
import { readNodeStatusAction } from "@/lib/connections-actions"
import { flowStages } from "@/lib/flow-stages"
import { latestSnapshot, transportLabels, type StampedSnapshot } from "@/lib/connections-model"
import { activeNodes, nextPartStatus, type PartStatus } from "@/lib/node-status"
import { emptyFlow, toFlow, validateFlow, type FlowDraft } from "@/lib/policy-model"
import { formatClock, formatRelative } from "@/lib/time"
import {
  channelStatusLabels,
  isLive,
  liveBetween,
  nodeAdjacencies,
  observeTopology,
  otherEnd,
  reportUnusableLabels,
  type Adjacency,
  type NodeReport,
  type TopologyObservation,
} from "@/lib/topology-model"
import { cn } from "@/lib/utils"

/** A result with the exact flow it answered; the form may have moved on. */
type Plan = { result: FlowPreviewResult; key: string; flow: FlowDraft }

/** The routing part of a combined preview, in the shape the path view renders. */
function routeResult(result: FlowPreviewResult): PreviewResult {
  if (result.status === "error") return result
  const { routing, diagnostics, revision, valid } = result.response
  return { status: "previewed", response: { decision: routing, diagnostics, revision, valid } }
}

type ConnectionsPageRead =
  | { ok: true; data: ConnectionsSnapshot; fetchedAt: number }
  | { ok: false; error: string; fetchedAt: number }

type Layers = { live: boolean; intent: boolean; plan: boolean }

const POLL_SECONDS = 12

/**
 * Network map over the real inventory, in three separate layers:
 * - live: neighbor channels fused from fresh agent self-reports, scoped to
 *   the saved connection plan (never a full mesh, never IP reachability);
 * - intent: every node pair the saved connection plan configures;
 * - plan: the controller compiling the *saved* policy snapshot this page
 *   was rendered with for one hypothetical flow, never a trace or a grant.
 * Connections, inventory and runtimes refresh together in one bounded poll;
 * the policy base only changes with a page refresh, which resets the plan.
 */
export function TopologyView({
  snapshot,
  nodes: renderedNodes,
  fetchedAt: renderedAt,
  initialConnections,
  initialRuntimes,
  initialNodeId,
  initialFlow,
}: {
  snapshot: PolicySnapshot
  nodes: NodeSummary[]
  fetchedAt: number
  initialConnections: ConnectionsPageRead
  initialRuntimes: InitialRuntimes
  initialNodeId: string | null
  initialFlow?: FlowDraft
}) {
  const routes = useRoutes()
  const router = useRouter()
  const networkId = useNetworkId()

  const [polledNodes, setPolledNodes] = useState<{ nodes: NodeSummary[]; fetchedAt: number } | null>(null)
  const [polledConnections, setPolledConnections] = useState<StampedSnapshot | null>(null)
  const [inventoryRead, setInventoryRead] = useState<PartStatus>({ at: renderedAt, error: null })
  const [connectionsRead, setConnectionsRead] = useState<PartStatus>({
    at: initialConnections.fetchedAt,
    error: initialConnections.ok ? null : initialConnections.error,
  })
  const poll = useCallback(async (): Promise<RuntimeReadResult> => {
    const result = await readNodeStatusAction(networkId)
    if (result.status === "error") return result
    const read = result.fetchedAt
    if (result.nodes.ok) {
      const list = result.nodes.data.nodes
      setPolledNodes((current) => (current && current.fetchedAt >= read ? current : { nodes: list, fetchedAt: read }))
    }
    setInventoryRead((current) => nextPartStatus(current, read, result.nodes.ok ? null : result.nodes.error))
    if (result.connections.ok) {
      const stamped = { snapshot: result.connections.data, readAt: read }
      setPolledConnections((current) => latestSnapshot(current, stamped))
    }
    setConnectionsRead((current) => nextPartStatus(current, read, result.connections.ok ? null : result.connections.error))
    return result.runtimes.ok
      ? { status: "loaded", snapshot: result.runtimes.data, fetchedAt: read }
      : { status: "error", message: result.runtimes.error }
  }, [networkId])
  const runtimes = useConnectionRuntimes(initialRuntimes, poll)

  const shownInventory =
    polledNodes !== null && polledNodes.fetchedAt > renderedAt ? polledNodes : { nodes: renderedNodes, fetchedAt: renderedAt }
  const nodes = shownInventory.nodes
  const fetchedAt = shownInventory.fetchedAt
  const connections = latestSnapshot(
    initialConnections.ok ? { snapshot: initialConnections.data, readAt: initialConnections.fetchedAt } : null,
    polledConnections
  )

  const active = activeNodes(nodes)
  const revokedCount = nodes.length - active.length
  const focusNodeId = initialNodeId || initialFlow?.sourceNodeId || null
  const [selectedId, setSelectedId] = useState<string | null>(
    focusNodeId && active.some((node) => node.id === focusNodeId)
      ? focusNodeId
      : (active[0]?.id ?? null)
  )
  const [flow, setFlow] = useState<FlowDraft>(() => initialFlow ?? {
    ...emptyFlow,
    sourceNodeId: initialNodeId && active.some((node) => node.id === initialNodeId) ? initialNodeId : "",
  })
  const [plan, setPlan] = useState<Plan | null>(null)
  const [layers, setLayers] = useState<Layers>({ live: true, intent: false, plan: true })
  const [pending, startPlan] = useTransition()
  const [refreshing, startRefresh] = useTransition()
  const seq = useRef(0)

  const flowKey = JSON.stringify(toFlow(flow))
  const flowError = validateFlow(flow)
  const response = plan?.result.status === "previewed" ? plan.result.response : null
  const flowChanged = plan !== null && plan.key !== flowKey
  const revisionMoved = response !== null && response.revision !== snapshot.revision
  const stale = flowChanged || revisionMoved
  // The global revision moves for any configuration change; the planner keeps
  // its own policy base and only says it may be behind.
  const configMoved = connections !== null && connections.snapshot.revision !== snapshot.revision

  const mapPlan: MapPlan | null =
    layers.plan && response && response.routing.path.length > 0
      ? {
          path: response.routing.path,
          external: planSteps(response.routing, externalLabel(plan?.flow ?? flow)).some(
            (step) => step.kind === "external"
          ),
          stale,
        }
      : null

  const runPlan = () => {
    const current = ++seq.current
    const key = flowKey
    const query = toFlow(flow)
    const submittedFlow = flow
    startPlan(async () => {
      const result = await previewFlowAction(networkId, query)
      if (current !== seq.current) return
      setPlan({ result, key, flow: submittedFlow })
    })
  }

  const observation: TopologyObservation | null = connections
    ? observeTopology(connections.snapshot, runtimes.snapshot, nodes, runtimes.now)
    : null
  // Without any runtime read, live adjacency is unknown, not absent.
  const liveKnown = observation !== null && runtimes.snapshot !== null
  const liveDegree = liveKnown
    ? new Map(active.map((node) => [node.id, nodeAdjacencies(observation, node.id).filter(isLive).length]))
    : null
  const name = (id: string) => nodes.find((node) => node.id === id)?.name ?? `${id.slice(0, 12)}…`
  const edgeTitle = (adjacency: Adjacency) =>
    `${name(adjacency.nodeIds[0])} — ${name(adjacency.nodeIds[1])}：${channelStatusLabels[adjacency.status]}。${adjacency.channels
      .map((channel) => channelText(channel, nodes))
      .join("；")}`
  const edgeLabel = (adjacency: Adjacency) => {
    const shown = adjacency.channels.filter((channel) => channel.status !== "unobserved")
    return [...new Set((shown.length > 0 ? shown : adjacency.channels).map((channel) => transportLabels[channel.transport]))].join(" / ")
  }
  const edges: MapEdge[] = []
  if (observation && layers.intent) {
    for (const adjacency of observation.adjacencies) {
      edges.push({
        key: `intent-${adjacency.key}`,
        kind: "intent",
        nodeIds: adjacency.nodeIds,
        label: edgeLabel(adjacency),
        title: `已配置：${edgeTitle(adjacency)}`,
      })
    }
  }
  if (observation && liveKnown && layers.live) {
    for (const adjacency of observation.adjacencies) {
      if (adjacency.status === "unobserved") continue
      edges.push({
        key: `live-${adjacency.key}`,
        kind: adjacency.status,
        nodeIds: adjacency.nodeIds,
        label: edgeLabel(adjacency),
        title: edgeTitle(adjacency),
      })
    }
  }
  const count = (status: Adjacency["status"]) =>
    observation?.adjacencies.filter((adjacency) => adjacency.status === status).length ?? 0
  const configuredPeers = new Set(observation?.adjacencies.flatMap((adjacency) => adjacency.nodeIds) ?? [])
  const isolated = observation ? active.filter((node) => !configuredPeers.has(node.id)) : []

  // Planned hops without a live adjacency: a plan is not reachability.
  const unbackedHops =
    liveKnown && response && !stale
      ? response.routing.path.flatMap((id, index) => {
          const next = response.routing.path[index + 1]
          if (next === undefined) return []
          return liveBetween(observation, id, next) ? [] : [`${name(id)} → ${name(next)}`]
        })
      : []

  const selected = selectedId ? active.find((node) => node.id === selectedId) : undefined
  const selectedAdjacencies = selected && observation ? nodeAdjacencies(observation, selected.id) : []
  const selectedReport = selected && observation ? observation.reports.get(selected.id) : undefined

  const readFailures = [
    runtimes.error ? { label: "运行状态", error: runtimes.error, lastGood: runtimes.fetchedAt } : null,
    connectionsRead.error !== null && (connections === null || connectionsRead.at > connections.readAt)
      ? { label: "连接配置", error: connectionsRead.error, lastGood: connections?.readAt ?? null }
      : null,
    inventoryRead.error !== null && inventoryRead.at > fetchedAt
      ? { label: "节点清单", error: inventoryRead.error, lastGood: fetchedAt }
      : null,
  ].filter((failure) => failure !== null)
  const toggle = (layer: keyof Layers) => (checked: boolean) => setLayers((current) => ({ ...current, [layer]: checked }))

  return (
    <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_22rem]">
      <div className="flex min-w-0 flex-col gap-4">
        <Panel
          data-testid="topology-map"
          title={
            <span className="flex flex-wrap items-center gap-2">
              网络拓扑
              {connections ? <Badge variant="outline">连接配置 r{connections.snapshot.revision}</Badge> : null}
              {liveKnown ? (
                <span data-testid="topology-live-summary" className="text-xs font-normal text-muted-foreground tabular-nums">
                  双向确认 {count("confirmed")}
                  {count("one_sided") > 0 ? ` · 单侧报告 ${count("one_sided")}` : ""}
                  {count("degraded") > 0 ? ` · 未认证 ${count("degraded")}` : ""}
                </span>
              ) : null}
            </span>
          }
          description="节点按名称环形排列，不代表地理位置。"
          actions={
            <span className="flex items-center gap-1.5">
              <span
                data-testid="topology-read-age"
                className="text-xs text-muted-foreground tabular-nums"
                title={`页面打开时每 ${POLL_SECONDS} 秒读取一次`}
              >
                {runtimes.fetchedAt !== null ? (
                  <>
                    <span className="hidden sm:inline">报告读取于 {formatClock(runtimes.fetchedAt)} · </span>
                    {formatRelative(new Date(runtimes.fetchedAt).toISOString(), runtimes.now) ?? "—"}
                  </>
                ) : (
                  "尚未读取运行报告"
                )}
              </span>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={runtimes.refresh}
                disabled={runtimes.refreshing}
                aria-label="刷新拓扑状态"
              >
                <RefreshCwIcon className={runtimes.refreshing ? "animate-spin" : undefined} />
              </Button>
            </span>
          }
        >
          {readFailures.length > 0 ? (
            <div
              role="status"
              data-testid="topology-read-error"
              className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b border-warning/40 bg-warning/5 px-4 py-2 text-xs"
            >
              <CircleAlertIcon className="size-3.5 shrink-0 text-warning" aria-hidden="true" />
              <span className="font-medium">
                {readFailures.map((failure) => failure.label).join("、")}读取失败
                {readFailures.every((failure) => failure.lastGood !== null)
                  ? `，显示 ${formatClock(Math.min(...readFailures.map((failure) => failure.lastGood ?? 0)))} 的数据；报告过期后不再显示为已认证`
                  : "，无法显示实际邻接"}
              </span>
              <span className="min-w-0 flex-1 truncate text-muted-foreground" title={readFailures.map((failure) => failure.error).join("\n")}>
                {readFailures[0]?.error}
              </span>
            </div>
          ) : null}
          <div className="flex flex-wrap items-center gap-x-5 gap-y-2 border-b px-4 py-2 text-xs">
            <LayerSwitch label="已认证邻接" checked={layers.live} onChange={toggle("live")} />
            <LayerSwitch label="配置意图" checked={layers.intent} onChange={toggle("intent")} />
            <LayerSwitch label="叠加规划路径" checked={layers.plan} onChange={toggle("plan")} />
          </div>
          {active.length === 0 ? (
            <p className="px-4 py-16 text-center text-sm text-muted-foreground">
              没有活动节点。
              <Link href={routes.addMachine} className="ml-1 underline underline-offset-4">
                注册节点
              </Link>
            </p>
          ) : (
            <div className="bg-grid-dots bg-canvas">
              <MeshMap
                nodes={active}
                selectedId={selectedId}
                sourceId={flow.sourceNodeId}
                targetId={flow.targetNodeId}
                plan={mapPlan}
                edges={edges}
                liveDegree={layers.live ? liveDegree : null}
                onSelect={setSelectedId}
              />
            </div>
          )}
          <div className="flex flex-col gap-1 border-t px-4 py-2 text-[0.6875rem] text-muted-foreground">
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
              <LineLegend className="stroke-success" label="双向确认" />
              <LineLegend className="stroke-warning" dash="7 5" label="单侧报告" />
              <LineLegend className="stroke-destructive/70" dash="2 4" label="未认证" />
              {layers.intent ? <LineLegend className="stroke-muted-foreground/50" dash="4 4" label="配置意图" /> : null}
              {layers.plan ? <LineLegend className="stroke-brand" dash="5 3" label="规划路径（假设流量）" /> : null}
              {revokedCount > 0 ? <span>已撤销 {revokedCount} 个节点未显示</span> : null}
            </div>
            <p>
              连线来自两端 agent 的认证探测报告，不代表 Mesh IP 流量。
              {liveKnown && count("unobserved") > 0 ? ` ${count("unobserved")} 条已配置邻接暂无可用报告。` : ""}
              {liveKnown && observation && observation.unmatchedReports > 0
                ? ` ${observation.unmatchedReports} 条报告与当前连接配置不对应，未显示。`
                : ""}
              {connections === null ? " 连接配置不可用，无法显示实际邻接。" : !liveKnown ? " 运行状态不可用，实际邻接未知。" : ""}
            </p>
          </div>
        </Panel>

        <Panel title="邻接列表" description="按已保存连接配置列出节点对与两端各自的报告。">
          {observation ? (
            <AdjacencyList
              adjacencies={observation.adjacencies}
              nodes={nodes}
              isolated={isolated}
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
          ) : (
            <p className="p-4 text-xs text-muted-foreground">连接配置不可用，无法列出邻接。</p>
          )}
        </Panel>

        <Panel title="节点列表">
          <ul className="flex flex-wrap gap-2 p-4">
            {active.map((node) => {
              const degree = liveDegree?.get(node.id)
              const configured = observation ? nodeAdjacencies(observation, node.id).length : null
              return (
                <li key={node.id}>
                  <button
                    type="button"
                    aria-pressed={selectedId === node.id}
                    onClick={() => setSelectedId(node.id)}
                    className={cn(
                      "flex max-w-60 min-w-0 flex-col rounded-lg border px-3 py-1.5 text-left transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring/30 focus-visible:outline-none",
                      selectedId === node.id && "border-brand/50 bg-brand/5",
                      (flow.sourceNodeId === node.id || flow.targetNodeId === node.id) && "border-brand"
                    )}
                  >
                    <span className="truncate text-xs font-medium">
                      {node.name}
                      {flow.sourceNodeId === node.id ? " · 来源" : ""}
                      {flow.targetNodeId === node.id ? " · 目标" : ""}
                    </span>
                    <span className="truncate font-mono text-[0.6875rem] text-muted-foreground">
                      {node.mesh_ipv4}

                    </span>
                    <span className="text-[0.6875rem] text-muted-foreground">
                      {configured === 0
                        ? "未配置邻接"
                        : degree === undefined
                          ? configured === null
                            ? "邻接未知"
                            : `已配置 ${configured} 个邻接`
                          : `已认证邻接 ${degree}/${configured}`}
                    </span>
                  </button>
                </li>
              )
            })}
          </ul>
        </Panel>
      </div>

      <div className="flex min-w-0 flex-col gap-4 xl:sticky xl:top-16">
        {selected ? (
          <Panel
            data-testid="topology-inspector"
            title={<NodeLink id={selected.id} nodes={nodes} />}
            bodyClassName="flex flex-col gap-3 p-4"
          >
            <dl className="grid grid-cols-[5.5rem_1fr] gap-x-3 gap-y-2 text-xs">
              <dt className="text-muted-foreground">Mesh IPv4</dt>
              <dd className="font-mono">{selected.mesh_ipv4}</dd>
              <dt className="text-muted-foreground">Mesh IPv6</dt>
              <dd className="font-mono break-all">{selected.mesh_ipv6}</dd>
              <dt className="text-muted-foreground">控制面联系</dt>
              <dd>
                <LastContact node={selected} fetchedAt={fetchedAt} />
              </dd>
              <dt className="text-muted-foreground">配置确认</dt>
              <dd>
                <ReceiptBadge node={selected} />
              </dd>
              <dt className="text-muted-foreground">运行报告</dt>
              <dd>
                <ReportState report={runtimes.snapshot === null ? undefined : selectedReport} now={runtimes.now} />
              </dd>
            </dl>
            <DiagnosticDetails title="拓扑节点技术详情"><p className="font-mono break-all">{selected.id}</p><p>Agent {selected.agent_version} · 配置 r{selected.configuration_revision} · 回执 r{selected.acknowledged_revision ?? "—"}</p></DiagnosticDetails>
            <div className="flex flex-col gap-2">
              <h3 className="text-xs font-medium">邻接</h3>
              {!observation ? (
                <p className="text-xs text-muted-foreground">连接配置不可用。</p>
              ) : selectedAdjacencies.length === 0 ? (
                <p className="text-xs text-muted-foreground">已保存的连接配置中没有与此节点相连的邻接。</p>
              ) : (
                <ul aria-label={`${selected.name} 的邻接`} className="flex flex-col gap-2">
                  {selectedAdjacencies.map((adjacency) => (
                    <li key={adjacency.key} data-testid="inspector-adjacency" data-status={adjacency.status} className="flex flex-col gap-1.5">
                      <span className="flex min-w-0 items-center gap-2 text-xs">
                        <NodeLink id={otherEnd(adjacency, selected.id)} nodes={nodes} />
                        <span className="ml-auto shrink-0">
                          <AdjacencyBadge status={liveKnown ? adjacency.status : "unobserved"} />
                        </span>
                      </span>
                      <ChannelRows adjacency={adjacency} nodes={nodes} perspective={selected.id} />
                    </li>
                  ))}
                </ul>
              )}
              <p className="text-[0.6875rem] text-muted-foreground">RTT 为各端自己的认证探测往返时间，不是单向延迟。</p>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setFlow({ ...flow, sourceNodeId: selected.id })}
                disabled={flow.sourceNodeId === selected.id}
              >
                设为来源
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  setFlow({ ...flow, targetNodeId: selected.id, domain: "", ip: "", country: "" })
                }
                disabled={flow.targetNodeId === selected.id}
              >
                设为目标
              </Button>
              <Link
                href={routes.machine(selected.id, "connections")}
                className={buttonVariants({ variant: "outline", size: "sm" })}
              >
                配置连接
              </Link>
              <Link
                href={routes.machine(selected.id)}
                className={buttonVariants({ variant: "ghost", size: "sm" })}
              >
                节点详情
                <ArrowRightIcon data-icon="inline-end" />
              </Link>
            </div>
          </Panel>
        ) : null}

        <Panel
          id="flow-check"
          data-testid="topology-flow-check"
          className="scroll-mt-16"
          title={
            <span className="flex flex-wrap items-center gap-2">
              检查流量
              <Badge variant="outline">已保存配置</Badge>
              {!snapshot.valid ? <Badge variant="destructive">策略无效</Badge> : null}
            </span>
          }
          description="按已保存的访问授权与路由规划一条流量；结果是规划，不是实测。"
          bodyClassName="flex flex-col gap-4 p-4"
        >
          <FlowForm nodes={nodes} flow={flow} onFlowChange={setFlow} disabled={pending} />
          <PreviewButton
            onClick={runPlan}
            disabled={pending || active.length === 0}
            pending={pending}
            blockedReason={flowError}
            label="规划路径"
          />
          {revisionMoved && response ? (
            <div className="flex flex-col items-start gap-2 rounded-lg border border-warning/40 bg-warning/5 px-3 py-2 text-xs">
              <span>
                预览基于 controller 当前的 r{response.revision}，而页面上的节点与策略读取于 r{snapshot.revision}；刷新后两者一致。
              </span>
              <RefreshPolicy refreshing={refreshing} onRefresh={() => startRefresh(() => router.refresh())} />
            </div>
          ) : configMoved && connections ? (
            <div
              data-testid="topology-config-moved"
              className="flex flex-col items-start gap-2 rounded-lg border border-warning/40 bg-warning/5 px-3 py-2 text-xs"
            >
              <span>
                Controller 配置已是 r{connections.snapshot.revision}，页面读取于 r{snapshot.revision}；预览总是使用 controller 当前的已保存配置。
              </span>
              <RefreshPolicy refreshing={refreshing} onRefresh={() => startRefresh(() => router.refresh())} />
            </div>
          ) : null}
          {unbackedHops.length > 0 ? (
            <p data-testid="plan-unbacked-hops" className="text-xs text-muted-foreground">
              规划中的 {unbackedHops.join("、")} 当前没有已认证邻接；规划路径不代表可达。
            </p>
          ) : null}
          <div className="border-t pt-4">
            <PreviewOutcome
              result={plan ? routeResult(plan.result) : null}
              nodes={nodes}
              flow={plan?.flow ?? flow}
              stale={stale}
              pending={pending}
              staleMessage={flowChanged ? "流量已修改，以下是上一次的规划。" : "基于较旧的策略版本。"}
              emptyMessage="选择源节点与目标，查看已保存策略为这条流量规划的逻辑路径。"
              onRuleClick={(ruleId) => router.push(routes.routing({ rule: ruleId }))}
            />
          </div>
          {response && !stale ? (
            <div className="flex flex-col gap-2 border-t pt-4">
              <h3 className="text-xs font-medium">这条流量的证据</h3>
              <FlowStages stages={flowStages(response, observation, liveKnown)} nodes={nodes} />
            </div>
          ) : null}
        </Panel>
      </div>
    </div>
  )
}

function RefreshPolicy({ refreshing, onRefresh }: { refreshing: boolean; onRefresh: () => void }) {
  return (
    <Button variant="outline" size="sm" disabled={refreshing} onClick={onRefresh}>
      <RefreshCwIcon data-icon="inline-start" className={refreshing ? "animate-spin" : undefined} />
      读取最新策略
    </Button>
  )
}

function ReportState({ report, now }: { report: NodeReport | undefined; now: number }) {
  if (!report) return <span className="text-muted-foreground">运行状态不可用</span>
  const reportedAt = report.observation?.reported_at ?? null
  const age = reportedAt ? formatRelative(reportedAt, now) : null
  return (
    <span className={cn(report.usable ? "text-success" : "text-muted-foreground")}>
      {report.usable ? "最近报告" : reportUnusableLabels[report.reason]}
      {age ? <span className="text-muted-foreground"> · {age}</span> : null}
    </span>
  )
}

function LayerSwitch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return (
    <label className="flex cursor-pointer items-center gap-2">
      <Switch size="sm" checked={checked} onCheckedChange={(value) => onChange(value)} />
      {label}
    </label>
  )
}

function LineLegend({ className, dash, label }: { className: string; dash?: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <svg aria-hidden="true" width="18" height="6" className="shrink-0">
        <line x1="1" y1="3" x2="17" y2="3" className={className} strokeWidth={2} strokeDasharray={dash} strokeLinecap="round" />
      </svg>
      {label}
    </span>
  )
}
