"use client"

import { useRoutes } from "@/components/shell/network-context"
import { useActionScope } from "@/components/network/network-workspace"

import type { ConnectionNode, ConnectionsSnapshot, EndpointDiscoveriesSnapshot, ManagedListenersSnapshot, NodeSummary } from "@meshway/control-client"
import {
  ActivityIcon,
  CircleAlertIcon,
  EllipsisIcon,
  RefreshCwIcon,
  ShieldOffIcon,
  ShieldCheckIcon,
  WaypointsIcon,
} from "lucide-react"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { useCallback, useState, type ReactNode } from "react"

import { PageHeader } from "@/components/shell/page-header"
import { ManagedDataplanePanel } from "@/components/nodes/managed-dataplane-panel"
import { NodeCarriers } from "@/components/nodes/node-carriers"
import { useManagedListenerDraftVersion } from "@/components/managed-listeners/draft-store"
import { ManagedListenerSheet, managedIdPrefix } from "@/components/managed-listeners/managed-listener-sheet"
import { NodeDnsPanel, type NodeDnsNetwork } from "@/components/nodes/node-dns-panel"
import { ConnectionsEditor } from "@/components/connections/connections-editor"
import { useConnectionsHasDraft } from "@/components/connections/draft-store"
import { ConnectionIdentityDetails, ConnectionReceipt } from "@/components/connections/node-connection-facts"
import { NodeRuntimePanel, RuntimeChip } from "@/components/connections/runtime-panel"
import { useConnectionRuntimes, type InitialRuntimes, type Runtimes } from "@/components/connections/use-runtimes"
import { ControllerUnavailable } from "@/components/common/controller-states"
import { CopyButton } from "@/components/common/copy-button"
import { Hint } from "@/components/common/hint"
import { Panel } from "@/components/common/panel"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { SectionHeader } from "@/components/common/section-header"
import { FailureNotice } from "@/components/common/failure-notice"
import { LastContact, ReceiptBadge, StatusBadge } from "@/components/nodes/node-status"
import { RevokeDialog } from "@/components/nodes/revoke-dialog"
import { Badge } from "@/components/ui/badge"
import { Button, buttonVariants } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
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
import { readManagedListenersAction } from "@/lib/managed-listener-actions"
import { createKey, editKey, type ManagedListenerDraft } from "@/lib/managed-listener-draft-registry"
import { fromSettings, managedById, newManagedForm, takenListenerIds } from "@/lib/managed-listener-model"
import {
  initialManagedRead,
  managedFreshness,
  recordManagedFailure,
  recordManagedSuccess,
  type ManagedReadState,
} from "@/lib/managed-read-state"
import { nextPartStatus, type PartStatus } from "@/lib/node-status"
import { formatClock, formatDateTime } from "@/lib/time"
import { topologyFlowHref } from "@/lib/flow-navigation"
import { diagnosticsHref } from "@/lib/connectivity-guidance"
import type { NodeOwner } from "@/lib/members-model"
import { roleLabels } from "@/lib/account-model"

export type NodeTab = "overview" | "connections"

/** Who owns the node, from the ownership and member reads. */
export type NodeOwnerRead = { ok: true; owner: NodeOwner | null } | { ok: false; error: string }

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
  owner,
  managed: initialManaged,
}: {
  owner: NodeOwnerRead
  node: NodeSummary
  fetchedAt: number
  initialTab: NodeTab
  /** Platform-managed listeners at render time. */
  managed: ControllerResult<ManagedListenersSnapshot>
  connections: ConnectionsPageRead
  controllerUrl: string
  initialRuntimes: InitialRuntimes
  discoveries: ControllerResult<EndpointDiscoveriesSnapshot>
  /** This node's tags, or an error when the tag snapshot could not be read. */
  tags: { ok: true; tags: string[] } | { ok: false; error: string }
  network: NodeDnsNetwork
}) {
  const routes = useRoutes()
  // The URL is the source of truth: history navigation can restore a cached
  // render whose server-provided tab is older than the entry's URL.
  const params = useSearchParams()
  const actionScope = useActionScope()
  const hasConnectionDraft = useConnectionsHasDraft()
  const tab: NodeTab = params.has("tab") ? (params.get("tab") === "connections" ? "connections" : "overview") : initialTab
  const [dialog, setDialog] = useState<"revoke" | null>(null)
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
  // Managed listeners: the shown snapshot and the latest read outcome are
  // tracked apart, so a failed read leaves a visibly dated cache.
  const [managedRead, setManagedRead] = useState<ManagedReadState>(() => initialManagedRead(initialManaged))
  const adoptManaged = useCallback((snapshot: ManagedListenersSnapshot, readAt: number) => {
    setManagedRead((current) => recordManagedSuccess(current, snapshot, readAt))
  }, [])
  const failManaged = useCallback((error: string, readAt: number) => {
    setManagedRead((current) => recordManagedFailure(current, error, readAt))
  }, [])
  const poll = useCallback(async (): Promise<RuntimeReadResult> => {
    const result = await readNodeStatusAction(actionScope)
    if (result.status === "error") return result
    const read = result.fetchedAt
    if (result.managed.ok) adoptManaged(result.managed.data, read)
    else failManaged(result.managed.error, read)
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
  }, [nodeId, actionScope, adoptManaged, failManaged])
  const runtimes = useConnectionRuntimes(initialRuntimes, poll)
  const managedSnapshot = managedRead.snapshot
  // Editing from a cached snapshot is allowed: the form's CAS base is that
  // revision, so a stale save conflicts instead of overwriting.
  const managedCachedAt = managedFreshness(managedRead) === "cached" ? managedRead.snapshotAt : null
  const revoked = node.revoked_at !== null
  const entry = latest.ok ? latest.data.nodes.find((candidate) => candidate.node.id === node.id) : undefined
  const savedScope = latest.ok ? nodeScope(fromPlan(latest.data.configuration), node.id) : null

  // Managed listener sheet: an unsaved form reopens when the configuration tab returns.
  const managedDrafts = useManagedListenerDraftVersion()
  const nodeDrafts = managedDrafts.forNode(nodeId).filter((draft) => draft.kind !== "enroll")
  const [sessionKey, setSessionKey] = useState<string | null>(() => nodeDrafts.find((draft) => draft.open)?.key ?? null)
  const [pristine, setPristine] = useState<ManagedListenerDraft | null>(null)
  const session = sessionKey === null ? null : (managedDrafts.get(sessionKey) ?? (pristine?.key === sessionKey ? pristine : null))
  const openCreate = () => {
    if (!managedSnapshot) return
    const key = createKey(nodeId)
    const kept = managedDrafts.get(key)
    if (kept) managedDrafts.put({ ...kept, open: true })
    else {
      const form = newManagedForm("vless_tcp", takenListenerIds(managedSnapshot, latest.ok ? latest.data : null), managedIdPrefix(latest.ok ? latest.data.nodes : [], nodeId))
      setPristine({ key, kind: "create", nodeId, baseRevision: managedSnapshot.revision, original: null, initial: form, form, idFollows: true, open: true })
    }
    setSessionKey(key)
  }
  const openEdit = (listenerId: string) => {
    if (!managedSnapshot) return
    const key = editKey(nodeId, listenerId)
    const kept = managedDrafts.get(key)
    if (kept) managedDrafts.put({ ...kept, open: true })
    else {
      const summary = managedById(managedSnapshot, listenerId)
      if (!summary) return
      const form = fromSettings(summary.settings)
      setPristine({ key, kind: "edit", nodeId, baseRevision: managedSnapshot.revision, original: summary.settings, initial: form, form, idFollows: false, open: true })
    }
    setSessionKey(key)
  }
  const closeSession = () => {
    const kept = sessionKey === null ? undefined : managedDrafts.get(sessionKey)
    if (kept) managedDrafts.put({ ...kept, open: false })
    setSessionKey(null)
  }
  const [retryingManaged, setRetryingManaged] = useState(false)
  const retryManaged = async () => {
    setRetryingManaged(true)
    const result = await readManagedListenersAction(actionScope)
    setRetryingManaged(false)
    if (result.status === "loaded") {
      adoptManaged(result.managed, result.fetchedAt)
      const connectionsRead = result.connections
      if (connectionsRead.ok) {
        const stamped = { snapshot: connectionsRead.data, readAt: result.fetchedAt }
        setAdopted((current) => latestSnapshot(current, stamped))
      }
    } else failManaged(result.message, result.fetchedAt)
  }

  const changeTab = (next: NodeTab) => {
    const url = new URL(window.location.href)
    url.searchParams.set("tab", next)
    window.history.replaceState(null, "", url)
  }

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        back={{ href: routes.machines(), label: "全部节点" }}
        title={node.name}
        status={<StatusBadge node={node} />}
        meta={
          <>
            <span className="inline-flex items-center gap-0.5 font-mono text-[0.8125rem] text-foreground">
              {node.mesh_ipv4}
              <CopyButton value={node.mesh_ipv4} label="复制 Mesh IPv4" iconOnly variant="ghost" />
            </span>
            <OwnerLabel owner={owner} />
          </>
        }
        actions={
          <>
            {!revoked ? (
              <Link href={topologyFlowHref(routes, { sourceNodeId: node.id })} className={buttonVariants({ variant: "outline" })}>
                <ShieldCheckIcon data-icon="inline-start" />
                检查访问
              </Link>
            ) : null}
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button variant="outline" size="icon" aria-label="更多节点操作" />}>
                <EllipsisIcon />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem render={<Link href={routes.topology({ node: node.id })} />}>
                  <WaypointsIcon />
                  在拓扑中查看
                </DropdownMenuItem>
                <DropdownMenuItem render={<Link href={diagnosticsHref(routes, node.id)} />}>
                  <ActivityIcon />
                  诊断此节点
                </DropdownMenuItem>
                {!revoked ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive" onClick={() => setDialog("revoke")}>
                      <ShieldOffIcon />
                      撤销节点…
                    </DropdownMenuItem>
                  </>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        }
      />

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

      <NodeFacts
        node={node}
        fetchedAt={fetchedAt}
        entry={entry}
        revision={latest.ok ? latest.data.revision : null}
        connectionsError={latest.ok ? null : latest.error}
        listeners={savedScope?.listeners.length ?? null}
        outgoing={savedScope?.outgoing.length ?? null}
        runtimes={runtimes}
        onOpenConfiguration={() => changeTab("connections")}
      />

      <Tabs value={tab} onValueChange={(value) => changeTab(value === "connections" ? "connections" : "overview")}>
        <TabsList variant="line" className="w-full justify-start border-b">
          <TabsTrigger value="overview" className="flex-none px-3">
            概览
          </TabsTrigger>
          <TabsTrigger value="connections" className="flex-none px-3">
            承载与配置
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="pt-4">
          <Overview node={node} connections={latest} runtimes={runtimes} tags={tags} network={network} owner={owner} />
        </TabsContent>

        <TabsContent value="connections" className="flex flex-col gap-4 pt-4">
          {latest.ok ? (
            <>
              <NodeCarriers
                nodeId={node.id}
                snapshot={latest.data}
                runtimes={runtimes}
                discoveries={discoveries}
                managed={managedRead}
                retryingManaged={retryingManaged}
                editable={!revoked}
                drafts={nodeDrafts}
                onCreateListener={openCreate}
                onEditListener={openEdit}
                onRetryManaged={() => {
                  if (!retryingManaged) void retryManaged()
                }}
              />
              {managedSnapshot && !revoked ? (
                <ManagedListenerSheet
                  session={session}
                  managed={managedSnapshot}
                  cachedAt={managedCachedAt}
                  connections={latest.data}
                  nodes={latest.data.nodes}
                  onClose={closeSession}
                  onSaved={(result) => {
                    adoptManaged(result.managed, result.fetchedAt)
                    if (result.connections.ok) {
                      const stamped = { snapshot: result.connections.data, readAt: result.fetchedAt }
                      setAdopted((current) => latestSnapshot(current, stamped))
                    }
                  }}
                  onReadFailed={failManaged}
                  onRead={(result) => {
                    adoptManaged(result.managed, result.fetchedAt)
                    if (result.connections.ok) {
                      const stamped = { snapshot: result.connections.data, readAt: result.fetchedAt }
                      setAdopted((current) => latestSnapshot(current, stamped))
                    }
                  }}
                />
              ) : null}
              <DiagnosticDetails title="连接身份与当前启用帮助">
                {entry ? <ConnectionIdentityDetails entry={entry} revision={latest.data.revision} /> : null}
                {!revoked ? <ManagedDataplanePanel /> : null}
              </DiagnosticDetails>
              <DiagnosticDetails title="高级手工连接配置" open={hasConnectionDraft || !latest.data.valid}>
                <SectionHeader id="node-connections-plan" title="连接配置（草稿）" description="保存的是意图；实际状态以上方报告为准。" />
                <ConnectionsEditor
                  snapshot={latest.data}
                  managedListenerIds={managedSnapshot ? new Set(managedSnapshot.listeners.map((listener) => listener.settings.id)) : null}
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
          <RevokeDialog node={node} open={dialog === "revoke"} onOpenChange={(open) => setDialog(open ? "revoke" : null)} />
        </>
      ) : null}
    </div>
  )
}

function Overview({
  node,
  connections,
  runtimes,
  tags,
  network,
  owner,
}: {
  node: NodeSummary
  connections: ConnectionsRead
  runtimes: Runtimes
  tags: { ok: true; tags: string[] } | { ok: false; error: string }
  network: NodeDnsNetwork
  owner: NodeOwnerRead
}) {
  const routes = useRoutes()
  return (
    <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)]">
      <div className="flex min-w-0 flex-col gap-4">
        {connections.ok ? (
          <NodeRuntimePanel nodeId={node.id} nodes={connections.data.nodes} runtimes={runtimes} />
        ) : (
          <FailureNotice title="连接配置读取失败，无法列出邻居" nextStep="稍后重试；持续失败时检查控制面。" error={connections.error} />
        )}
      </div>

      <div className="flex min-w-0 flex-col gap-4">
        <Panel title="身份与地址" bodyClassName="px-4 py-1 sm:px-5">
          <dl className="divide-y">
            <Row label="Mesh IPv4" hint="隧道内地址，不是其他节点建连用的对外地址。">
              <CopyValue value={node.mesh_ipv4} label="复制 IPv4" />
            </Row>
            <Row label="Mesh IPv6">
              <CopyValue value={node.mesh_ipv6} label="复制 IPv6" />
            </Row>
            <Row label="归属">
              <span className="flex items-center justify-between gap-2">
                <OwnerLabel owner={owner} full />
                <Link href={routes.members} className={buttonVariants({ variant: "ghost", size: "sm" })}>
                  成员
                </Link>
              </span>
            </Row>
            <Row label="标签" hint="管理员维护的分组，供访问控制引用。">
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
                  <Link href={routes.tags({ node: node.id })} className={buttonVariants({ variant: "ghost", size: "sm" })}>
                    修改
                  </Link>
                ) : null}
              </span>
            </Row>
            <Row label="Agent 版本">
              <span className="font-mono">{node.agent_version}</span>
            </Row>
            <Row label="注册于">{formatDateTime(node.registered_at)}</Row>
            {node.revoked_at !== null ? (
              <Row label="撤销于">
                <span className="font-mono">{formatDateTime(node.revoked_at)}</span>
              </Row>
            ) : null}
          </dl>
          <div className="py-3">
            <DiagnosticDetails title="技术详情">
              <dl>
                <Row label="节点 ID"><CopyValue value={node.id} label="复制节点 ID" /></Row>
                {node.country !== null ? (
                  <Row label="旧版国家标注" hint="v2 路由国家条件仍可能引用；不是位置，也不再在节点上维护。">
                    <span className="font-mono">{node.country}</span>
                  </Row>
                ) : null}
              </dl>
            </DiagnosticDetails>
          </div>
        </Panel>

        <NodeDnsPanel node={node} network={network} />
      </div>
    </div>
  )
}

/**
 * Four independent facts about the node, never merged into one "online"
 * state: control-plane contact, configuration receipt, the agent's own
 * runtime report, and the saved carriers.
 */
function NodeFacts({
  node,
  fetchedAt,
  entry,
  revision,
  connectionsError,
  listeners,
  outgoing,
  runtimes,
  onOpenConfiguration,
}: {
  node: NodeSummary
  fetchedAt: number
  entry: ConnectionNode | undefined
  revision: number | null
  connectionsError: string | null
  listeners: number | null
  outgoing: number | null
  runtimes: Runtimes
  onOpenConfiguration: () => void
}) {
  const ready = entry ? readiness(entry) : null
  return (
    <dl data-slot="node-facts" className="grid overflow-hidden rounded-xl border bg-card shadow-xs sm:grid-cols-2 xl:grid-cols-4 [&>div]:border-b sm:[&>div:nth-child(odd)]:border-r xl:[&>div]:border-b-0 xl:[&>div:not(:last-child)]:border-r">
      <FactCell label="控制面联系" hint="controller 最近一次接受注册或心跳；不代表在线或隧道连通。">
        <LastContact node={node} fetchedAt={fetchedAt} />
      </FactCell>
      <FactCell label="IP / 路由配置" hint="Agent 已校验并保存该版本；不代表数据面已生效。">
        <ReceiptBadge node={node} />
      </FactCell>
      <FactCell label="运行报告" hint="Agent 自报的运行阶段与邻居认证探测。">
        <RuntimeChip observation={runtimeFor(runtimes.snapshot, node.id)} now={runtimes.now} readFailed={runtimes.snapshot === null} />
      </FactCell>
      <FactCell label="承载">
        {connectionsError !== null ? (
          <span className="text-xs text-destructive">读取失败</span>
        ) : !entry ? (
          <span className="text-xs text-muted-foreground">未出现在连接配置中</span>
        ) : (
          <button
            type="button"
            onClick={onOpenConfiguration}
            className="inline-flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 rounded text-left text-[0.8125rem] outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/40"
          >
            <span>
              {listeners === 0 ? "无监听，只拨出" : `${listeners ?? 0} 个监听`} · {outgoing ?? 0} 个主动连接
            </span>
            {entry && revision !== null ? <ConnectionReceipt entry={entry} revision={revision} /> : null}
            {ready !== null && ready !== "ready" ? <span className="text-xs text-warning">{readinessLabels[ready]}</span> : null}
          </button>
        )}
      </FactCell>
    </dl>
  )
}

function FactCell({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5 px-4 py-3">
      <dt className="flex items-center gap-1 text-xs text-muted-foreground">
        {label}
        {hint ? <Hint label={label}>{hint}</Hint> : null}
      </dt>
      <dd className="min-w-0 text-[0.8125rem]">{children}</dd>
    </div>
  )
}

function OwnerLabel({ owner, full = false }: { owner: NodeOwnerRead; full?: boolean }) {
  if (!owner.ok) return <span className="text-destructive" title={owner.error}>归属读取失败</span>
  const value = owner.owner
  if (value === null) return full ? <span className="text-muted-foreground">未知</span> : null
  if (value.kind === "member") {
    return (
      <span className="inline-flex min-w-0 items-center gap-1.5">
        <span className="truncate">{value.email}</span>
        {full ? <span className="text-xs text-muted-foreground">{roleLabels[value.role]}</span> : null}
      </span>
    )
  }
  if (value.kind === "unassigned") return <span className="text-warning">未分配</span>
  if (value.kind === "former") return <span className="text-muted-foreground">已离开的成员</span>
  return <span className="text-muted-foreground">未知</span>
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[6.5rem_minmax(0,1fr)] items-start gap-3 py-2.5 sm:grid-cols-[8rem_minmax(0,1fr)]">
      <dt className="flex items-center gap-1 pt-px text-xs text-muted-foreground">
        {label}
        {hint ? <Hint label={label}>{hint}</Hint> : null}
      </dt>
      <dd className="min-w-0 text-[0.8125rem]">{children}</dd>
    </div>
  )
}

function CopyValue({ value, label }: { value: string; label: string }) {
  return (
    <span className="flex min-w-0 items-center justify-between gap-2">
      <code className="min-w-0 font-mono text-[0.8125rem] break-all">{value}</code>
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

