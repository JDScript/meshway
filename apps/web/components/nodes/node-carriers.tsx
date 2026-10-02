"use client"

import type {
  ConnectionListener,
  ConnectionNeighborObservation,
  ConnectionNode,
  ConnectionsSnapshot,
  EndpointDiscoveriesSnapshot,
  ManagedListenerSummary,
  ManagedListenersSnapshot,
} from "@meshway/control-client"
import { ArrowRightIcon, PencilIcon, PlusIcon, RefreshCwIcon } from "lucide-react"
import type { ReactNode } from "react"

import { CopyButton } from "@/components/common/copy-button"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { FailureNotice } from "@/components/common/failure-notice"
import { Hint } from "@/components/common/hint"
import { Panel } from "@/components/common/panel"
import { StatusBadge, type Tone } from "@/components/common/status-badge"
import { EndpointDiscoveryReport, useEndpointDiscoveries } from "@/components/connections/endpoint-discovery-panel"
import type { Runtimes } from "@/components/connections/use-runtimes"
import { Button } from "@/components/ui/button"
import {
  nodeCarrierView,
  observedStateLabels,
  peerProbe,
  type ListenerView,
  type ObservedAddress,
  type OutgoingView,
  type PeerCheck,
  type SocketFacts,
} from "@/lib/carrier-view-model"
import { neighborLabel, reasonLabels, runtimeFor, runtimeNextSteps } from "@/lib/connection-runtime-model"
import type { ControllerResult } from "@/lib/controller"
import { nodeName, transportLabels } from "@/lib/connections-model"
import { discoveryLabel, discoveryStateLabels } from "@/lib/endpoint-discovery-model"
import type { ManagedListenerDraft } from "@/lib/managed-listener-draft-registry"
import { certificatePreview, describeSettings, managedFor, presetLabels, presetOf, referencingCandidates } from "@/lib/managed-listener-model"
import { managedFreshness, type ManagedReadState } from "@/lib/managed-read-state"
import { formatClock } from "@/lib/time"
import { formatDateTime } from "@/lib/time"
import { cn } from "@/lib/utils"

/**
 * The node's listeners and dial sockets, one row each: saved configuration,
 * what the agent bound, external addresses with their source and expiry,
 * and what each peer verified. Raw report fields stay folded below.
 */
/** How the managed snapshot relates to the connection plan shown on this page. */
type ManagedContext = {
  /** The managed read is current and describes the same revision as the plan. */
  sameRevision: boolean
  freshness: ReturnType<typeof managedFreshness>
  revision: number | null
  planRevision: number
}

export function NodeCarriers({
  nodeId,
  snapshot,
  runtimes,
  discoveries: initialDiscoveries,
  managed,
  editable,
  drafts,
  onCreateListener,
  onEditListener,
  onRetryManaged,
  retryingManaged,
}: {
  nodeId: string
  snapshot: ConnectionsSnapshot
  runtimes: Runtimes
  discoveries: ControllerResult<EndpointDiscoveriesSnapshot>
  /** Platform-managed listener intent with its read state; a failed read never hides observed facts. */
  managed: ManagedReadState
  retryingManaged: boolean
  /** Active node: managed listeners can be added and edited. */
  editable: boolean
  /** This node's unsaved managed-listener forms. */
  drafts: ManagedListenerDraft[]
  onCreateListener: () => void
  onEditListener: (listenerId: string) => void
  onRetryManaged: () => void
}) {
  const discoveries = useEndpointDiscoveries(initialDiscoveries)
  const report = discoveries.shown.ok ? discoveries.shown.data.nodes.find((node) => node.node_id === nodeId) : undefined
  // One clock for every lease on this view; both sources only move forward.
  const now = Math.max(runtimes.now, discoveries.clock)
  const view = nodeCarrierView({
    plan: snapshot.configuration,
    revision: snapshot.revision,
    nodeId,
    runtime: runtimeFor(runtimes.snapshot, nodeId),
    discovery: report,
    now,
  })
  const nodes = snapshot.nodes
  const serverStatus = discoveries.shown.ok ? discoveryLabel(report, now) : "观测读取失败"
  // The server may call a report current while it describes another plan
  // revision or runtime generation than the one shown here.
  const discoveryStatus = serverStatus === discoveryStateLabels.observed && !view.discoveryCurrent ? "与当前配置 / 运行代际不一致（历史）" : serverStatus
  const managedSnapshot: ManagedListenersSnapshot | null = managed.snapshot
  const freshness = managedFreshness(managed)
  const context: ManagedContext = {
    freshness,
    revision: managedSnapshot?.revision ?? null,
    planRevision: snapshot.revision,
    sameRevision: freshness === "current" && managedSnapshot?.revision === snapshot.revision,
  }
  const managedOwn = managedSnapshot ? managedFor(managedSnapshot, nodeId) : []
  const managedById = new Map(managedOwn.map((summary) => [summary.settings.id, summary]))
  const planListeners = new Map(snapshot.configuration.listeners.map((listener) => [listener.id, listener]))
  const disabled = managedOwn.filter((summary) => !summary.settings.enabled)
  const createDraft = drafts.find((draft) => draft.kind === "create")
  const editDraft = (id: string) => drafts.some((draft) => draft.kind === "edit" && draft.original?.id === id)
  const listenerCount = view.listeners.length + disabled.length

  return (
    <div id="node-carriers" data-testid="node-carriers" className="flex flex-col gap-4">
      <Panel
        title="监听"
        description={listenerCount === 0 ? "没有监听：此节点只主动拨出，其他节点不能拨入。" : "其他节点可拨入的入口。"}
        actions={
          <span className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
            <span>UDP 地址观测：{discoveryStatus}</span>
            <Hint label="UDP 地址观测">
              STUN 观测来自 Agent 实际承载的 UDP socket，是未经对端验证的候选；不代表可达，也不适用于 TCP / VLESS。
            </Hint>
            <Button variant="ghost" size="icon-sm" aria-label="刷新地址观测" disabled={discoveries.refreshing} onClick={discoveries.refresh}>
              <RefreshCwIcon className={discoveries.refreshing ? "animate-spin" : undefined} />
            </Button>
          </span>
        }
      >
        {freshness !== "current" ? (
          <div data-testid="managed-read-failure" data-freshness={freshness} className="border-b px-4 py-3 sm:px-5">
            <FailureNotice
              title={
                freshness === "failed"
                  ? "托管监听读取失败"
                  : `托管监听读取失败，显示 ${formatClock(managed.snapshotAt)} 的缓存（r${managedSnapshot?.revision ?? "—"}）`
              }
              nextStep={
                freshness === "failed"
                  ? "暂时不能添加或编辑监听；下方仍显示已保存连接与运行报告。"
                  : "缓存可能已过时；仍可基于它编辑，保存时以该版本做 CAS 校验，版本变化会冲突而不会覆盖。"
              }
              error={managed.status.error ?? "未读取"}
              actions={
                <Button variant="outline" size="sm" disabled={retryingManaged} onClick={onRetryManaged}>
                  <RefreshCwIcon data-icon="inline-start" className={retryingManaged ? "animate-spin" : undefined} />
                  重试
                </Button>
              }
            />
          </div>
        ) : null}
        {!discoveries.shown.ok ? (
          <div className="border-b px-4 py-3 sm:px-5">
            <FailureNotice title="地址观测读取失败" nextStep="点击刷新地址观测重试。" error={discoveries.shown.error} />
          </div>
        ) : null}
        {view.listeners.length > 0 ? (
          <ul aria-label="监听" className="divide-y">
            {view.listeners.map((listener) => (
              <ListenerRow
                key={listener.id}
                listener={listener}
                nodes={nodes}
                now={now}
                ownership={
                  <Ownership
                    summary={managedById.get(listener.id)}
                    plan={planListeners.get(listener.id)}
                    context={context}
                    editable={editable}
                    hasDraft={editDraft(listener.id)}
                    onEdit={() => onEditListener(listener.id)}
                  />
                }
              />
            ))}
          </ul>
        ) : null}
        {disabled.length > 0 ? (
          <ul aria-label="已停用的监听" className="divide-y border-t">
            {disabled.map((summary) => (
              <DisabledListenerRow
                key={summary.settings.id}
                summary={summary}
                snapshot={snapshot}
                context={context}
                editable={editable}
                hasDraft={editDraft(summary.settings.id)}
                onEdit={() => onEditListener(summary.settings.id)}
              />
            ))}
          </ul>
        ) : null}
        {editable && managedSnapshot ? (
          <div className="border-t px-4 py-3 sm:px-5">
            <Button variant="outline" size="sm" onClick={onCreateListener} data-testid="managed-listener-create">
              {createDraft ? <PencilIcon data-icon="inline-start" /> : <PlusIcon data-icon="inline-start" />}
              {createDraft ? "继续添加监听（有未保存的修改）" : "添加监听"}
            </Button>
          </div>
        ) : null}
      </Panel>

      <Panel
        title="主动连接"
        description={view.outgoing.length === 0 ? "没有主动连接。" : "本节点拨出的连接；拨号 socket 的观测只属于该 socket，不是监听。"}
      >
        {view.outgoing.length > 0 ? (
          <ul aria-label="主动连接" className="divide-y">
            {view.outgoing.map((candidate) => (
              <OutgoingRow key={candidate.id} candidate={candidate} nodes={nodes} now={now} />
            ))}
          </ul>
        ) : null}
      </Panel>

      {view.orphans.length > 0 ? (
        <Panel title="其他已上报 socket" description="报告中的资源不在当前已保存配置里（例如已删除），只作历史参考。">
          <ul className="divide-y">
            {view.orphans.map((orphan) => (
              <li key={`${orphan.kind}:${orphan.resourceId}:${orphan.facts.socketId}`} className="flex flex-col gap-2 px-4 py-3 sm:px-5">
                <span className="text-[0.8125rem] font-medium break-all">
                  {orphan.kind === "listener" ? "监听" : "主动连接"} · <code className="font-mono">{orphan.resourceId}</code>
                </span>
                <Socket facts={orphan.facts} now={now} kind={orphan.kind === "listener" ? "listener" : "dial"} />
              </li>
            ))}
          </ul>
        </Panel>
      ) : null}

      <DiagnosticDetails title="地址观测报告与启用帮助">
        <EndpointDiscoveryReport discoveries={discoveries} nodeId={nodeId} embedded fenced={view.discoveryCurrent} now={now} />
      </DiagnosticDetails>
    </div>
  )
}

function ListenerRow({ listener, nodes, now, ownership }: { listener: ListenerView; nodes: ConnectionNode[]; now: number; ownership: ReactNode }) {
  const runtime = listener.runtime
  const tone: Tone = !runtime ? "neutral" : runtime.state === "bound" ? "success" : "danger"
  const state = !runtime ? "未报告" : runtime.state === "bound" ? "已监听" : `监听失败${reasonLabels[runtime.reason] ? `：${reasonLabels[runtime.reason]}` : ""}`
  const sockets = listener.udp.kind === "sockets" ? listener.udp.sockets : []
  return (
    <li data-testid="carrier-listener" data-listener-id={listener.id} className="flex flex-col gap-3 px-4 py-3.5 sm:px-5">
      <RowTitle transport={transportLabels[listener.transport]} id={listener.id} status={<StatusBadge tone={tone} dot>{state}</StatusBadge>} />
      {runtime && runtime.state === "error" && runtime.reason !== "none" ? (
        <p role="status" className="text-xs text-destructive">{runtimeNextSteps[runtime.reason]}</p>
      ) : null}
      <dl className="grid gap-x-6 gap-y-3 md:grid-cols-2">
        <Fact label="绑定配置">
          <Address value={listener.bind} label={`复制 ${listener.id} 的绑定配置`} />
        </Fact>
        <Fact label="实际绑定" hint="Agent 报告的 socket 本地地址；只有 UDP socket 上报具体地址。">
          <BoundSockets sockets={sockets} copyLabel={`复制 ${listener.id} 的实际绑定`} empty={runtime?.state === "bound" ? "已监听，地址未上报" : "未上报"} />
        </Fact>
        <Fact label="对外候选" className="md:col-span-2" hint="对端可尝试的地址，按来源区分。管理员发布的地址只是意图；STUN 观测是未验证候选。">
          <ul className="flex flex-col gap-1.5">
            {listener.advertised.length === 0 && sockets.every((socket) => socket.observed.length === 0) ? (
              <li className="text-muted-foreground">没有对外地址：其他节点无法找到这个监听。</li>
            ) : null}
            {listener.advertised.map((address) => (
              <Candidate key={`a:${address}`} address={address} source="管理员发布" tone="info" state="已保存" copyable copyLabel={`复制 ${listener.id} 的对外地址 ${address}`} />
            ))}
            {sockets.flatMap((socket) => socket.observed.map((observed) => (
              <Observed key={`s:${socket.socketId}:${observed.observer}:${observed.observedAt}`} observed={observed} now={now} source="STUN 观测" />
            )))}
            {listener.udp.kind === "not_applicable" ? (
              <li className="text-muted-foreground">{transportLabels[listener.transport]} 不做 UDP 地址观测。</li>
            ) : null}
          </ul>
        </Fact>
        {ownership}
        <Fact label="对端验证" className="md:col-span-2" hint="Agent 报告的邻居认证探测结果，按配置的拨入关系逐个列出。">
          {listener.incoming.length === 0 ? (
            <span className="text-muted-foreground">没有节点配置拨入。</span>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {listener.incoming.map((check) => (
                <PeerCheckRow key={check.candidateId} check={check} nodes={nodes} direction="incoming" />
              ))}
            </ul>
          )}
        </Fact>
      </dl>
      <SocketDetails sockets={sockets} />
    </li>
  )
}

function OutgoingRow({ candidate, nodes, now }: { candidate: OutgoingView; nodes: ConnectionNode[]; now: number }) {
  const sockets = candidate.udp.kind === "sockets" ? candidate.udp.sockets : []
  const target = candidate.targetNodeId === null ? "监听已不存在" : nodeName(nodes, candidate.targetNodeId)
  return (
    <li data-testid="carrier-outgoing" data-candidate-id={candidate.id} className="flex flex-col gap-3 px-4 py-3.5 sm:px-5">
      <RowTitle
        transport={candidate.transport ? transportLabels[candidate.transport] : "未知承载"}
        id={candidate.id}
        status={<span className="text-xs text-muted-foreground tabular-nums">优先级 {candidate.priority}</span>}
      >
        <span className="inline-flex min-w-0 items-center gap-1 text-[0.8125rem] text-muted-foreground">
          <ArrowRightIcon className="size-3.5 shrink-0" aria-hidden="true" />
          <span className="truncate text-foreground">{target}</span>
        </span>
      </RowTitle>
      <dl className="grid gap-x-6 gap-y-3 md:grid-cols-2">
        <Fact label="拨号目标">
          <Address value={candidate.endpoint} label={`复制 ${candidate.id} 的拨号目标`} />
        </Fact>
        <Fact label="本地 socket" hint="Agent 报告的拨号 socket 本地地址；只有 UDP socket 上报。">
          {candidate.udp.kind === "not_applicable" ? (
            <span className="text-muted-foreground">未知 · {candidate.transport ? transportLabels[candidate.transport] : "该承载"} 拨号 socket 暂不上报</span>
          ) : (
            <BoundSockets sockets={sockets} copyLabel={`复制 ${candidate.id} 的本地 socket`} empty="未上报" />
          )}
        </Fact>
        {candidate.udp.kind !== "not_applicable" ? (
          <Fact label="出站映射" className="md:col-span-2" hint="拨号 socket 被 STUN 观测到的公网地址；仅属于该 socket，不是可拨入的监听。">
            {sockets.every((socket) => socket.observed.length === 0) ? (
              <span className="text-muted-foreground">未上报</span>
            ) : (
              <ul className="flex flex-col gap-1.5">
                {sockets.flatMap((socket) => socket.observed.map((observed) => (
                  <Observed key={`${socket.socketId}:${observed.observer}:${observed.observedAt}`} observed={observed} now={now} source="STUN 观测" />
                )))}
              </ul>
            )}
          </Fact>
        ) : null}
        <Fact label="对端验证" className="md:col-span-2" hint="Agent 报告的邻居认证探测结果。">
          <ul className="flex flex-col gap-1.5">
            <PeerCheckRow check={candidate.check} nodes={nodes} direction="outgoing" />
          </ul>
        </Fact>
      </dl>
      <SocketDetails sockets={sockets} />
    </li>
  )
}

/**
 * Who owns a listener's configuration: the platform (editable here, with its
 * public material status) or a manual / legacy plan entry (read-only here).
 */
function Ownership({
  summary,
  plan,
  context,
  editable,
  hasDraft,
  onEdit,
}: {
  summary: ManagedListenerSummary | undefined
  plan: ConnectionListener | undefined
  context: ManagedContext
  editable: boolean
  hasDraft: boolean
  onEdit: () => void
}) {
  if (summary) {
    return (
      <Fact label="平台托管" className="md:col-span-2" hint="由控制面保存并下发；保存表示意图，Agent 回执、实际绑定与对端验证分别显示。">
        <ManagedFacts summary={summary} context={context} editable={editable} hasDraft={hasDraft} onEdit={onEdit} />
      </Fact>
    )
  }
  // Only a current managed read of the same revision can say a listener is not managed.
  const markedManaged = plan !== undefined && plan.credential_ref === `managed.${plan.id}`
  if (markedManaged || !context.sameRevision) {
    return (
      <Fact label="配置来源" className="md:col-span-2">
        <span data-testid="listener-source-unknown" className="text-muted-foreground">
          {markedManaged ? "平台托管 · 托管设置未读取到" : "未确定"}
          {context.freshness === "failed"
            ? "（托管监听读取失败）"
            : context.freshness === "cached"
              ? `（托管监听读取失败，缓存 r${context.revision ?? "—"} 中没有它）`
              : `（托管监听 r${context.revision ?? "—"} 与连接配置 r${context.planRevision} 不是同一版本）`}
        </span>
      </Fact>
    )
  }
  const reference = plan?.credential_ref ?? null
  return (
    <Fact label="配置来源" className="md:col-span-2" hint="旧手工连接配置；不在这里编辑。">
      <span data-testid="manual-listener" className="flex flex-col gap-1">
        <span className="flex flex-wrap items-center gap-2">
          <StatusBadge tone="neutral">手工配置（旧）</StatusBadge>
          {reference ? (
            <span className="text-xs text-muted-foreground">
              引用节点本地 profile <code className="font-mono">{reference}</code>
            </span>
          ) : null}
        </span>
        <span className="text-xs text-muted-foreground">在下方「高级手工连接配置」查看；不建议在节点上手工维护 profile。</span>
      </span>
    </Fact>
  )
}

function ManagedFacts({
  summary,
  context,
  editable,
  hasDraft,
  onEdit,
}: {
  summary: ManagedListenerSummary
  context: ManagedContext
  editable: boolean
  hasDraft: boolean
  onEdit: () => void
}) {
  const settings = summary.settings
  const vless = settings.vless
  return (
    <span data-testid="managed-listener" data-listener-id={settings.id} className="flex flex-col gap-1.5">
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="font-medium">{presetLabels[presetOf(settings)]}</span>
        {vless ? (
          <span className="text-xs text-muted-foreground">
            SNI <code className="font-mono">{vless.server_name}</code>
            {vless.stream === "websocket" ? (
              <>
                {" · "}路径 <code className="font-mono">{vless.websocket_path}</code>
                {" · "}Host <code className="font-mono">{vless.websocket_host}</code>
              </>
            ) : null}
          </span>
        ) : null}
        {editable ? (
          <Button variant="outline" size="sm" className="ml-auto" onClick={onEdit} aria-label={`编辑监听 ${settings.id}`}>
            <PencilIcon data-icon="inline-start" />
            {hasDraft ? "继续编辑" : "编辑"}
          </Button>
        ) : null}
      </span>
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
        {!context.sameRevision ? (
          <StatusBadge tone={context.freshness === "cached" ? "warning" : "neutral"}>
            {context.freshness === "cached" ? `缓存 · 托管设置 r${context.revision ?? "—"}` : `托管设置 r${context.revision ?? "—"} · 连接配置 r${context.planRevision}`}
          </StatusBadge>
        ) : null}
        {summary.compiled_capability_available ? (
          <StatusBadge tone="success">承载能力可用</StatusBadge>
        ) : (
          <StatusBadge tone="warning" title="节点未报告或不支持这种承载（含 VLESS 的 TCP / WebSocket 底层）；已保存的意图会保留，能力可用后才下发执行。">
            当前承载能力不可用
          </StatusBadge>
        )}
        {hasDraft ? <StatusBadge tone="warning">有未保存的修改</StatusBadge> : null}
        {vless ? (
          <span className="text-muted-foreground">
            材料 v{summary.material_version}
            {summary.certificate_sha256 ? (
              <>
                {" · "}证书 SHA-256 <code className="font-mono" title={summary.certificate_sha256}>{certificatePreview(summary.certificate_sha256)}</code>
              </>
            ) : null}
          </span>
        ) : null}
        {vless ? (
          <Hint label="证书">平台自签证书，拨号方按下发的证书精确信任；不是公网 CA 签发，也不证明已安装或可达。</Hint>
        ) : null}
      </span>
    </span>
  )
}

function DisabledListenerRow({
  summary,
  snapshot,
  context,
  editable,
  hasDraft,
  onEdit,
}: {
  summary: ManagedListenerSummary
  snapshot: ConnectionsSnapshot
  context: ManagedContext
  editable: boolean
  hasDraft: boolean
  onEdit: () => void
}) {
  const settings = summary.settings
  const references = referencingCandidates(snapshot, settings.id, [])
  return (
    <li data-testid="carrier-listener-disabled" data-listener-id={settings.id} className="flex flex-col gap-2 px-4 py-3.5 sm:px-5">
      <RowTitle transport={presetLabels[presetOf(settings)]} id={settings.id} status={
          <span className="flex items-center gap-1.5">
            {!context.sameRevision ? <StatusBadge tone="warning">托管设置 r{context.revision ?? "—"}{context.freshness === "cached" ? " · 缓存" : ""}</StatusBadge> : null}
            <StatusBadge tone="neutral">已停用</StatusBadge>
          </span>
        } />
      <p className="text-xs text-muted-foreground">{describeSettings(settings)} · 设置与材料保留，执行已撤回。</p>
      {references.length > 0 ? (
        <p role="status" className="text-xs text-warning">
          仍有 {references.length} 个拨号候选引用它（{references.map((candidate) => candidate.id).join("、")}）：已保存的连接配置无效，直到重新启用或在高级入口中修复。
        </p>
      ) : null}
      {editable ? (
        <Button variant="outline" size="sm" className="self-start" onClick={onEdit} aria-label={`编辑监听 ${settings.id}`}>
          <PencilIcon data-icon="inline-start" />
          {hasDraft ? "继续编辑" : "编辑 / 重新启用"}
        </Button>
      ) : null}
    </li>
  )
}

function RowTitle({ transport, id, status, children }: { transport: string; id: string; status: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1.5">
      <span className="rounded-md border bg-muted/50 px-1.5 py-0.5 text-[0.6875rem] font-medium">{transport}</span>
      <code className="min-w-0 truncate font-mono text-[0.8125rem] font-medium">{id}</code>
      {children}
      <span className="ml-auto">{status}</span>
    </div>
  )
}

function Fact({ label, hint, children, className }: { label: string; hint?: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-1", className)}>
      <dt className="flex items-center gap-1 text-xs text-muted-foreground">
        {label}
        {hint ? <Hint label={label}>{hint}</Hint> : null}
      </dt>
      <dd className="min-w-0 text-[0.8125rem]">{children}</dd>
    </div>
  )
}

function Address({ value, label, disabled = false }: { value: string; label: string; disabled?: boolean }) {
  return (
    <span className="flex min-w-0 items-center gap-1">
      <code className={cn("min-w-0 font-mono break-all", disabled && "text-muted-foreground line-through decoration-muted-foreground/50")}>{value}</code>
      <CopyButton value={value} label={label} iconOnly variant="ghost" disabled={disabled} />
    </span>
  )
}

function Candidate({
  address,
  source,
  tone,
  state,
  detail,
  copyable,
  copyLabel,
}: {
  address: string | null
  source: string
  tone: Tone
  state: string
  detail?: ReactNode
  copyable: boolean
  copyLabel: string
}) {
  return (
    <li className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
      {address ? <Address value={address} label={copyLabel} disabled={!copyable} /> : <span className="text-muted-foreground">无映射地址</span>}
      <span className="text-xs text-muted-foreground">{source}</span>
      <StatusBadge tone={tone}>{state}</StatusBadge>
      {detail ? <span className="text-xs text-muted-foreground tabular-nums">{detail}</span> : null}
    </li>
  )
}

function Observed({ observed, now, source }: { observed: ObservedAddress; now: number; source: string }) {
  const remaining = Math.max(0, Math.ceil((Date.parse(observed.expiresAt) - now) / 1000))
  const tone: Tone = observed.state === "candidate" ? "warning" : "neutral"
  const label = observed.state === "failed" ? `无映射 · ${observed.outcome}` : observedStateLabels[observed.state]
  return (
    <Candidate
      address={observed.address}
      source={source}
      tone={tone}
      state={label}
      detail={observed.state === "candidate" ? `${remaining} 秒后过期` : observed.state === "expired" ? `过期于 ${formatDateTime(observed.expiresAt)}` : undefined}
      copyable={observed.state === "candidate"}
      copyLabel={observed.state === "candidate" ? `复制观测地址 ${observed.address ?? ""}` : "复制历史观测地址（失效）"}
    />
  )
}

function PeerCheckRow({ check, nodes, direction }: { check: PeerCheck; nodes: ConnectionNode[]; direction: "incoming" | "outgoing" }) {
  const name = check.peerNodeId === "" ? "未知节点" : nodeName(nodes, check.peerNodeId)
  return (
    <li className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
      <span className="font-medium">{name}</span>
      <span className="text-xs text-muted-foreground">
        {direction === "incoming" ? `经 ${check.endpoint} 拨入` : `拨向 ${check.endpoint}`}
      </span>
      {check.reports.length === 0 ? (
        <StatusBadge tone="neutral">未报告</StatusBadge>
      ) : (
        check.reports.map((peer, index) => <PeerReport key={index} peer={peer} />)
      )}
    </li>
  )
}

function PeerReport({ peer }: { peer: ConnectionNeighborObservation }) {
  const tone: Tone = peer.state === "authenticated" ? "success" : peer.state === "unreachable" ? "danger" : "neutral"
  const probe = peerProbe(peer)
  return (
    <span className="inline-flex items-center gap-1.5">
      <StatusBadge tone={tone} dot>{neighborLabel(peer)}</StatusBadge>
      {peer.state !== "authenticated" ? null : probe.fresh ? (
        <span className="font-mono text-xs text-muted-foreground tabular-nums">{probe.rtt ?? "RTT 未测得"}</span>
      ) : (
        <span className="text-xs text-muted-foreground">最近无新鲜探测</span>
      )}
    </span>
  )
}

/**
 * Agent-reported socket addresses. Only sockets of a current report are a
 * current binding (and copyable); others are labelled history.
 */
function BoundSockets({ sockets, copyLabel, empty }: { sockets: SocketFacts[]; copyLabel: string; empty: string }) {
  const current = sockets.filter((socket) => socket.current)
  const history = sockets.filter((socket) => !socket.current)
  return (
    <span className="flex flex-col gap-1">
      {current.length === 0 ? <span className="text-muted-foreground">{empty}</span> : null}
      {current.map((socket) => <Address key={socket.socketId} value={socket.boundAddress} label={copyLabel} />)}
      {history.map((socket) => (
        <span key={socket.socketId} data-testid="carrier-history-bind" className="flex flex-wrap items-center gap-1.5">
          <Address value={socket.boundAddress} label="复制历史绑定地址（失效）" disabled />
          <StatusBadge tone="neutral">历史报告</StatusBadge>
        </span>
      ))}
    </span>
  )
}

function Socket({ facts, now, kind }: { facts: SocketFacts; now: number; kind: "listener" | "dial" }) {
  return (
    <div className="flex flex-col gap-1.5 text-[0.8125rem]">
      <span className="flex flex-wrap items-center gap-2">
        <span className="text-xs text-muted-foreground">{kind === "listener" ? "实际绑定" : "本地 socket"}</span>
        <Address value={facts.boundAddress} label="复制历史绑定地址" disabled />
      </span>
      <ul className="flex flex-col gap-1.5">
        {facts.observed.map((observed) => (
          <Observed key={`${observed.observer}:${observed.observedAt}`} observed={observed} now={now} source="STUN 观测" />
        ))}
      </ul>
    </div>
  )
}

function SocketDetails({ sockets }: { sockets: SocketFacts[] }) {
  if (sockets.length === 0) return null
  return (
    <DiagnosticDetails title="socket 技术详情">
      <ul className="flex flex-col gap-2">
        {sockets.map((socket) => (
          <li key={socket.socketId} className="flex flex-col gap-1 font-mono">
            <span>generation {socket.generation} · socket {socket.socketId} · {socket.source}</span>
            {socket.observed.map((observed) => (
              <span key={`${observed.observer}:${observed.observedAt}`} className="text-muted-foreground">
                observer {observed.observer} · {observed.outcome} · STUN RTT {observed.rttMs === null ? "未测得" : `${observed.rttMs} ms`} · observed {formatDateTime(observed.observedAt)} · expires {formatDateTime(observed.expiresAt)}
              </span>
            ))}
          </li>
        ))}
      </ul>
    </DiagnosticDetails>
  )
}

