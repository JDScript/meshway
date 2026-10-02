"use client"

import { useRoutes } from "@/components/shell/network-context"
import { useNetworkId } from "@/components/network/network-workspace"

import type { ConnectionsSnapshot } from "@meshway/control-client"
import {
  ArrowRightLeftIcon,
  CircleAlertIcon,
  EarIcon,
  HistoryIcon,
  PhoneOutgoingIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SaveIcon,
  TriangleAlertIcon,
} from "lucide-react"
import Link from "next/link"
import { useEffect, useMemo, useRef, useState, useTransition, type ReactNode } from "react"

import { CandidateSheet } from "@/components/connections/candidate-sheet"
import {
  ConnectionDiagnosticList,
  diagnosticsFor,
} from "@/components/connections/connection-diagnostics"
import { CandidateRow, ListenerRow, problemCount } from "@/components/connections/connection-rows"
import { useConnectionsDraftStore, type Owner } from "@/components/connections/draft-store"
import { IdentityPanel } from "@/components/connections/identity-panel"
import { ListenerSheet } from "@/components/connections/listener-sheet"
import type { Runtimes } from "@/components/connections/use-runtimes"
import { DraftActionBar } from "@/components/common/draft-action-bar"
import { Panel } from "@/components/common/panel"
import { Metric, StatusSummary } from "@/components/common/status-summary"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import { Spinner } from "@/components/ui/spinner"
import type { ConnectionsSaveResult } from "@/lib/action-state"
import {
  candidateRowNeighbors,
  listenerRowObservation,
  reasonLabels,
} from "@/lib/connection-runtime-model"
import {
  reloadConnectionsAction,
  saveConnectionsAction,
} from "@/lib/connections-actions"
import {
  anyRoleBlocker,
  candidateChange,
  canServe,
  CONNECTION_LIMITS,
  dialRole,
  dialRoleLabel,
  draftKey,
  fromCandidateForm,
  fromPlan,
  listenerChange,
  newCandidateForm,
  newListenerForm,
  nodeName,
  nodeScope,
  planKey,
  removedCount,
  resolveConnectionNode,
  toCandidateForm,
  toPlan,
  transports,
  validateConnections,
  type CandidateDraft,
  type CandidateForm,
  type CandidateSession,
  type ConnectionsDraft,
  type ListenerDraft,
  type ListenerSession,
  type SessionFixed,
  type WorkingSheet,
} from "@/lib/connections-model"
import { cn } from "@/lib/utils"

type Confirm =
  | { kind: "reload" }
  | { kind: "revert" }
  | { kind: "delete-listener"; key: string; references: number }

/**
 * Connection control intent: who listens where, and who dials which
 * listener. One draft covers the whole plan; the per-node view only filters
 * it, so saving from a node page never drops other nodes' items. The editing
 * base (`base`) only changes on a successful save or an explicit reload; a
 * fresher page snapshot or runtime refresh is reported, never adopted.
 * Saving freezes editing until the controller answers.
 */
export function ConnectionsEditor({
  snapshot,
  scopeNodeId,
  runtimes,
  onSnapshot,
}: {
  snapshot: ConnectionsSnapshot
  /** Per-node view, or null for the whole network. */
  scopeNodeId: string | null
  /** Live self-reports, displayed only; never part of the draft. */
  runtimes: Runtimes | null
  /**
   * A snapshot the controller returned for a successful save or an explicit
   * reload, so a parent that remounts this editor (e.g. tabs) passes it back
   * instead of its older page snapshot. Never called for background reads.
   */
  onSnapshot?: (snapshot: ConnectionsSnapshot, fetchedAt: number) => void
}) {
  const routes = useRoutes()
  const drafts = useConnectionsDraftStore()
  const networkId = useNetworkId()
  const [restored] = useState(() => drafts.read())
  const [restoredNotice, setRestoredNotice] = useState(restored !== null)
  const [base, setBase] = useState(restored?.base ?? snapshot)
  const [draft, setDraft] = useState<ConnectionsDraft>(
    () => restored?.draft ?? fromPlan(snapshot.configuration)
  )
  const [save, setSave] = useState<ConnectionsSaveResult | null>(null)
  const [reloadError, setReloadError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<Confirm | null>(null)
  // Unapplied sheet edits come back too: the sheet reopens with them.
  const [working, setWorking] = useState<WorkingSheet | null>(restored?.sheet ?? null)
  const [listenerSheet, setListenerSheet] = useState<{ session: ListenerSession; open: boolean } | null>(
    () =>
      restored?.sheet?.kind === "listener"
        ? { open: true, session: { ...restored.sheet.session, working: restored.sheet.form } }
        : null
  )
  const [candidateSheet, setCandidateSheet] = useState<{ session: CandidateSession; open: boolean } | null>(
    () =>
      restored?.sheet?.kind === "candidate"
        ? { open: true, session: { ...restored.sheet.session, working: restored.sheet.form } }
        : null
  )
  const [highlight, setHighlight] = useState<string | null>(null)
  const [saving, startSave] = useTransition()
  const [reloading, startReload] = useTransition()
  const nextKey = useRef(restored?.nextKey ?? 0)
  const sessionId = useRef(restored?.nextSession ?? 0)
  const draftRef = useRef(draft)
  // This mount's claim on the shared store, and the current reload request.
  // Completions from an unmounted editor, or a superseded reload, never touch
  // the store or a newer editor's work.
  const owner = useRef<Owner | null>(null)
  const reloadGeneration = useRef(0)

  useEffect(() => {
    const claimed = drafts.claim()
    owner.current = claimed
    const generation = reloadGeneration
    return () => {
      generation.current += 1
      owner.current = null
      drafts.release(claimed)
    }
  }, [drafts])

  useEffect(() => {
    draftRef.current = draft
  }, [draft])

  useEffect(() => {
    if (highlight === null) return
    const timer = window.setTimeout(() => setHighlight(null), 2500)
    return () => window.clearTimeout(timer)
  }, [highlight])

  // Identity and receipts come from the freshest read (receipts change
  // without a revision bump); the CAS base does not move.
  const latest = snapshot.revision >= base.revision ? snapshot : base
  const nodes = latest.nodes
  const behind = snapshot.revision > base.revision

  const currentKey = draftKey(draft)
  const dirty = currentKey !== planKey(base.configuration)
  const issues = useMemo(() => validateConnections(draft, nodes), [draft, nodes])
  const locked = saving || reloading

  useEffect(() => {
    if (dirty || working !== null) {
      drafts.write(owner.current, {
        base,
        draft,
        sheet: working,
        nextKey: nextKey.current,
        nextSession: sessionId.current,
      })
    } else {
      drafts.clear(owner.current)
    }
  }, [dirty, working, base, draft, drafts])

  const usage = useMemo(() => {
    const counts = new Map<string, { listeners: number; candidates: number }>()
    const bump = (id: string, field: "listeners" | "candidates") => {
      if (id === "") return
      const current = counts.get(id) ?? { listeners: 0, candidates: 0 }
      counts.set(id, { ...current, [field]: current[field] + 1 })
    }
    for (const listener of draft.listeners) bump(listener.nodeId, "listeners")
    for (const candidate of draft.candidates) bump(candidate.dialerNodeId, "candidates")
    return counts
  }, [draft])

  const listenerById = (id: string) => draft.listeners.find((listener) => listener.id.trim() === id)
  const dialersOf = (listenerId: string) =>
    draft.candidates
      .filter((candidate) => candidate.listenerId === listenerId)
      .map((candidate) => nodeName(nodes, candidate.dialerNodeId))

  const scope = scopeNodeId ? nodeScope(draft, scopeNodeId) : null
  const scopeRef = scopeNodeId ? resolveConnectionNode(nodes, scopeNodeId) : null
  const scopeActive = scopeRef?.kind === "active" ? scopeRef.entry : null
  const canAccept = scopeNodeId
    ? scopeActive !== null && transports.some((t) => canServe(scopeActive, t, "accept"))
    : nodes.some((entry) => transports.some((t) => canServe(entry, t, "accept")))
  const canDial = scopeNodeId
    ? scopeActive !== null && transports.some((t) => canServe(scopeActive, t, "dial"))
    : nodes.some((entry) => transports.some((t) => canServe(entry, t, "dial")))

  const updateDraft = (update: (current: ConnectionsDraft) => ConnectionsDraft) => {
    setDraft(update)
    setSave(null)
  }

  // --- sheets --------------------------------------------------------------

  /** Ends the open session; its unapplied edits go, the outer draft stays. */
  const closeListenerSheet = () => {
    setWorking(null)
    setListenerSheet((current) => (current ? { ...current, open: false } : null))
  }
  const closeCandidateSheet = () => {
    setWorking(null)
    setCandidateSheet((current) => (current ? { ...current, open: false } : null))
  }
  const opened = <T,>(session: { id: number; mode: "new" | "edit"; initial: T; fixed?: SessionFixed }) => ({
    id: session.id,
    mode: session.mode,
    initial: session.initial,
    fixed: session.fixed ?? null,
  })
  const trackListenerForm = (form: ListenerDraft | null) => {
    const session = listenerSheet?.session
    if (!session) return
    setWorking(form === null ? null : { kind: "listener", session: opened(session), form })
  }
  const trackCandidateForm = (form: CandidateForm | null) => {
    const session = candidateSheet?.session
    if (!session) return
    setWorking(form === null ? null : { kind: "candidate", session: opened(session), form })
  }

  // --- listeners -----------------------------------------------------------

  const newListener = () => {
    const key = `new-${nextKey.current++}`
    sessionId.current += 1
    setWorking(null)
    setListenerSheet({
      open: true,
      session: {
        id: sessionId.current,
        mode: "new",
        initial: newListenerForm(key, draft, nodes, scopeNodeId),
        fixed: scopeNodeId ? "node" : null,
      },
    })
  }
  const editListener = (listener: ListenerDraft) => {
    sessionId.current += 1
    setWorking(null)
    setListenerSheet({
      open: true,
      session: {
        id: sessionId.current,
        mode: "edit",
        initial: listener,
        fixed: scopeNodeId && listener.nodeId === scopeNodeId ? "node" : null,
      },
    })
  }
  const applyListener = (listener: ListenerDraft) => {
    const session = listenerSheet?.session
    if (!session) return
    const previousId = session.mode === "edit" ? session.initial.id.trim() : null
    updateDraft((current) => {
      const exists = current.listeners.some((existing) => existing.key === listener.key)
      const listeners = exists
        ? current.listeners.map((existing) => (existing.key === listener.key ? listener : existing))
        : [...current.listeners, listener]
      // A rename inside the draft keeps its candidates pointing at it.
      const candidates =
        previousId !== null && previousId !== listener.id
          ? current.candidates.map((candidate) =>
              candidate.listenerId === previousId ? { ...candidate, listenerId: listener.id } : candidate
            )
          : current.candidates
      return { listeners, candidates }
    })
    setHighlight(listener.key)
    closeListenerSheet()
  }
  const removeListener = (key: string) => {
    setConfirm(null)
    updateDraft((current) => ({
      ...current,
      listeners: current.listeners.filter((listener) => listener.key !== key),
    }))
  }
  const requestRemoveListener = (listener: ListenerDraft) => {
    const references = dialersOf(listener.id.trim()).length
    if (references > 0) setConfirm({ kind: "delete-listener", key: listener.key, references })
    else removeListener(listener.key)
  }

  // --- candidates ----------------------------------------------------------

  const newCandidate = (context: { dialerNodeId?: string; targetNodeId?: string; listenerId?: string }, fixed: SessionFixed) => {
    const key = `new-${nextKey.current++}`
    sessionId.current += 1
    setWorking(null)
    setCandidateSheet({
      open: true,
      session: { id: sessionId.current, mode: "new", initial: newCandidateForm(key, draft, nodes, context), fixed },
    })
  }
  const dialFromScope = () => newCandidate({ dialerNodeId: scopeNodeId ?? undefined }, scopeNodeId ? "dialer" : null)
  const allowIntoListener = (listener: ListenerDraft) =>
    newCandidate({ listenerId: listener.id.trim() }, scopeNodeId && listener.nodeId === scopeNodeId ? "target" : null)
  const editCandidate = (candidate: CandidateDraft) => {
    const form = toCandidateForm(candidate, draft.listeners)
    sessionId.current += 1
    setWorking(null)
    setCandidateSheet({
      open: true,
      session: {
        id: sessionId.current,
        mode: "edit",
        initial: form,
        fixed:
          scopeNodeId === null
            ? null
            : candidate.dialerNodeId === scopeNodeId
              ? "dialer"
              : form.targetNodeId === scopeNodeId
                ? "target"
                : null,
      },
    })
  }
  const applyCandidate = (candidate: CandidateDraft) => {
    updateDraft((current) => {
      const exists = current.candidates.some((existing) => existing.key === candidate.key)
      return {
        ...current,
        candidates: exists
          ? current.candidates.map((existing) => (existing.key === candidate.key ? candidate : existing))
          : [...current.candidates, candidate],
      }
    })
    setHighlight(candidate.key)
    closeCandidateSheet()
  }
  const removeCandidate = (key: string) =>
    updateDraft((current) => ({
      ...current,
      candidates: current.candidates.filter((candidate) => candidate.key !== key),
    }))

  // --- save / reload -------------------------------------------------------

  const runSave = () => {
    const expected = base.revision
    const plan = toPlan(draft)
    const sentKey = currentKey
    const saver = owner.current
    startSave(async () => {
      const result = await saveConnectionsAction(networkId, expected, plan)
      if (result.status === "saved") onSnapshot?.(result.snapshot, result.fetchedAt)
      if (!drafts.owns(saver)) {
        // This editor unmounted meanwhile. Only kept work still based on
        // exactly what was saved moves onto the saved snapshot, and only if
        // no newer editor owns the store.
        if (result.status === "saved") drafts.adoptOrphanSave(expected, sentKey, result.snapshot)
        return
      }
      setSave(result)
      if (result.status === "saved") {
        setRestoredNotice(false)
        setBase(result.snapshot)
        // Adopt the controller's canonical (sorted, normalized) plan only
        // when the draft is still exactly what was sent.
        if (draftKey(draftRef.current) === sentKey) setDraft(fromPlan(result.snapshot.configuration))
      }
    })
  }

  const runReload = () => {
    setConfirm(null)
    const generation = ++reloadGeneration.current
    const loader = owner.current
    startReload(async () => {
      const result = await reloadConnectionsAction(networkId)
      // Superseded, or this editor unmounted: newer work must survive.
      if (generation !== reloadGeneration.current || !drafts.owns(loader)) return
      if (result.status === "error") {
        setReloadError(result.message)
        return
      }
      setReloadError(null)
      onSnapshot?.(result.snapshot, result.fetchedAt)
      // The administrator confirmed discarding the draft.
      setWorking(null)
      setListenerSheet((current) => (current ? { ...current, open: false } : null))
      setCandidateSheet((current) => (current ? { ...current, open: false } : null))
      setRestoredNotice(false)
      setBase(result.snapshot)
      setDraft(fromPlan(result.snapshot.configuration))
      setSave(null)
      setHighlight(null)
    })
  }
  const requestReload = () => (dirty ? setConfirm({ kind: "reload" }) : runReload())
  const revertDraft = () => {
    setConfirm(null)
    setRestoredNotice(false)
    setDraft(fromPlan(base.configuration))
    setSave(null)
  }

  // --- derived row state ---------------------------------------------------

  const issuesFor = (key: string) => issues.filter((issue) => issue.itemKey === key)
  const planIssues = issues.filter((issue) => issue.itemKey === null)
  const listenerIds = new Set(base.configuration.listeners.map((listener) => listener.id))
  const candidateIds = new Set(base.configuration.candidates.map((candidate) => candidate.id))
  const orphanDiagnostics = base.diagnostics.filter(
    (diagnostic) =>
      diagnostic.item_id === null ||
      !(
        (diagnostic.field.startsWith("listeners") && listenerIds.has(diagnostic.item_id)) ||
        (diagnostic.field.startsWith("candidates") && candidateIds.has(diagnostic.item_id))
      )
  )

  const changedListeners = draft.listeners.filter(
    (listener) => listenerChange(listener, base.configuration) !== "unchanged"
  ).length
  const changedCandidates = draft.candidates.filter(
    (candidate) => candidateChange(candidate, base.configuration) !== "unchanged"
  ).length
  const removed = removedCount(draft, base.configuration)
  const changeSummary = [
    changedListeners > 0 ? `${changedListeners} 个监听新增或修改` : null,
    changedCandidates > 0 ? `${changedCandidates} 个主动连接新增或修改` : null,
    removed.listeners > 0 ? `删除 ${removed.listeners} 个监听` : null,
    removed.candidates > 0 ? `删除 ${removed.candidates} 个主动连接` : null,
  ].filter((part) => part !== null)

  // Live badges only for items unchanged against the saved base, from a
  // fresh report of exactly that base revision. Edited items show their
  // draft badge instead; old proof is never projected onto a new target.
  const listenerRow = (listener: ListenerDraft, showOwner: boolean) => {
    const change = listenerChange(listener, base.configuration)
    const observed = runtimes
      ? listenerRowObservation(runtimes.snapshot, runtimes.now, base.revision, listener, change === "unchanged")
      : undefined
    return (
      <ListenerRow
        key={listener.key}
        listener={listener}
        nodes={nodes}
        showOwner={showOwner}
        change={change}
        problems={problemCount(
          change,
          issuesFor(listener.key),
          diagnosticsFor(base.diagnostics, "listeners", listener.id.trim()).length
        )}
        dialers={dialersOf(listener.id.trim())}
        runtime={observed ? { state: observed.state, reason: reasonLabels[observed.reason] } : null}
        highlighted={highlight === listener.key}
        disabled={locked}
        onEdit={() => editListener(listener)}
        onDial={() => allowIntoListener(listener)}
        onDelete={() => requestRemoveListener(listener)}
      />
    )
  }
  const candidateRow = (candidate: CandidateDraft) => {
    const change = candidateChange(candidate, base.configuration)
    const listener = listenerById(candidate.listenerId)
    const neighbors = runtimes
      ? candidateRowNeighbors(
          runtimes.snapshot,
          runtimes.now,
          base.revision,
          candidate,
          listener,
          change === "unchanged" && listener !== undefined && listenerChange(listener, base.configuration) === "unchanged"
        )
      : []
    return (
      <CandidateRow
        key={candidate.key}
        candidate={candidate}
        listener={listener}
        nodes={nodes}
        change={change}
        problems={problemCount(
          change,
          issuesFor(candidate.key),
          diagnosticsFor(base.diagnostics, "candidates", candidate.id.trim()).length
        )}
        role={dialRoleLabel(dialRole(candidate, draft))}
        neighbors={neighbors}
        highlighted={highlight === candidate.key}
        disabled={locked}
        onEdit={() => editCandidate(candidate)}
        onDelete={() => removeCandidate(candidate.key)}
      />
    )
  }

  const editingListener = listenerSheet?.session.initial
  const editingCandidate = candidateSheet?.session.initial
  const listenerSheetDiagnostics =
    editingListener && listenerSheet?.session.mode === "edit"
      ? diagnosticsFor(base.diagnostics, "listeners", editingListener.id.trim())
      : []
  const candidateSheetDiagnostics =
    editingCandidate && candidateSheet?.session.mode === "edit"
      ? diagnosticsFor(base.diagnostics, "candidates", editingCandidate.id.trim())
      : []

  const addListenerButton = (label: string, variant: "default" | "outline" = "default") => (
    <Button
      size="sm"
      variant={variant}
      disabled={locked || !canAccept || draft.listeners.length >= CONNECTION_LIMITS.listeners}
      onClick={newListener}
    >
      <PlusIcon data-icon="inline-start" />
      {label}
    </Button>
  )
  const addCandidateButton = (label: string, variant: "default" | "outline" = "default") => (
    <Button
      size="sm"
      variant={variant}
      disabled={
        locked ||
        !canDial ||
        draft.listeners.length === 0 ||
        draft.candidates.length >= CONNECTION_LIMITS.candidates
      }
      onClick={dialFromScope}
    >
      <PlusIcon data-icon="inline-start" />
      {label}
    </Button>
  )

  return (
    <div data-testid="connections-editor" data-scope={scopeNodeId ?? "all"} className="flex flex-col gap-4">
      <StatusStrip
        base={base}
        scopeNodeId={scopeNodeId}
        reloading={reloading}
        disabled={locked}
        onReload={requestReload}
      />

      {!base.valid ? (
        <Alert variant="destructive">
          <TriangleAlertIcon />
          <AlertTitle>已保存的连接配置当前无效</AlertTitle>
          <AlertDescription className="flex flex-col gap-2">
            <p>
              配置引用了已撤销、未发布公钥或不再具备能力的节点等，所有节点收到的连接意图为空。原配置保留在下方，修复后保存即可。
            </p>
            <ConnectionDiagnosticList label="已保存配置的诊断" diagnostics={base.diagnostics} showItem />
          </AlertDescription>
        </Alert>
      ) : null}

      {restoredNotice && restored ? (
        <Alert>
          <HistoryIcon />
          <AlertTitle>已恢复未保存的草稿</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>
              之前的修改{restored.sheet ? "（包括未应用的表单内容）" : ""}仍保留在本次会话中，编辑基础仍为 r{restored.base.revision}。刷新浏览器、退出登录会丢弃草稿。
            </p>
            <Button variant="ghost" size="sm" onClick={() => setRestoredNotice(false)}>
              知道了
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {behind ? (
        <Alert>
          <TriangleAlertIcon />
          <AlertTitle>Controller 已有更新的配置版本 r{snapshot.revision}</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>
              草稿基于 r{base.revision}，编辑基础不会自动前移，此时保存会因版本冲突被拒绝。节点公钥、路由或 IP 变化也会推进版本。
            </p>
            <Button variant="outline" size="sm" disabled={locked} onClick={requestReload}>
              载入最新版本{dirty ? "（先确认放弃草稿）" : ""}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}
      {reloadError ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
          <span className="break-all">{reloadError}</span>
        </p>
      ) : null}

      {scopeNodeId && scope ? (
        <>
          {scopeRef && scopeActive === null ? (
            <Alert variant="destructive">
              <TriangleAlertIcon />
              <AlertTitle>{scopeRef.kind === "revoked" ? "节点已撤销" : "节点不在连接清单中"}</AlertTitle>
              <AlertDescription>
                与它相关的配置仍显示在下方，便于删除或修复；不能再为它新增监听或主动连接。
              </AlertDescription>
            </Alert>
          ) : scopeActive && anyRoleBlocker(scopeActive, "dial") && anyRoleBlocker(scopeActive, "accept") ? (
            <Alert>
              <CircleAlertIcon />
              <AlertTitle>这个节点还不能参与连接：{anyRoleBlocker(scopeActive, "dial")}</AlertTitle>
              <AlertDescription>
                在节点上用带承载实现的构建发布连接身份后即可配置（命令见「连接配置」页的节点列表）。
              </AlertDescription>
            </Alert>
          ) : null}

          <Panel
            title={<PanelTitle icon={<EarIcon />} label="接收其他节点连接" count={scope.listeners.length} />}
            description="本节点在哪个端口等待连接，以及其他节点通过什么地址找到它。"
            actions={addListenerButton("新增监听")}
          >
            {scope.listeners.length === 0 ? (
              <Empty className="py-8">
                <EmptyHeader>
                  <EmptyTitle>没有监听</EmptyTitle>
                  <EmptyDescription>
                    不需要公网地址也能加入网络：只要它主动连接一个有监听的节点，建立的通道双方都能复用。
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : (
              <ul aria-label="本节点的监听" className="divide-y">
                {scope.listeners.map((listener) => listenerRow(listener, false))}
              </ul>
            )}
            {scope.incoming.length > 0 ? (
              <div className="border-t">
                <h3 className="px-4 pt-3 pb-1 text-xs font-medium text-muted-foreground">
                  由其他节点主动连接到这里（{scope.incoming.length}）
                </h3>
                <ul aria-label="其他节点主动连接本节点" className="divide-y">
                  {scope.incoming.map(candidateRow)}
                </ul>
              </div>
            ) : null}
          </Panel>

          <Panel
            title={<PanelTitle icon={<PhoneOutgoingIcon />} label="主动连接节点" count={scope.outgoing.length} />}
            description="本节点主动拨号哪些节点的监听；同一目标的多个入口按优先级主备。"
            actions={addCandidateButton("连接到节点")}
          >
            {scope.outgoing.length === 0 ? (
              <Empty className="py-8">
                <EmptyHeader>
                  <EmptyTitle>没有主动连接</EmptyTitle>
                  <EmptyDescription>
                    {listeningNodesExcept(draft, scopeNodeId)
                      ? "选择一个正在接收连接的节点，并使用它公布的地址。"
                      : "还没有其他节点在接收连接。先在另一个节点上新增监听。"}
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : (
              <ul aria-label="本节点的主动连接" className="divide-y">
                {scope.outgoing.map(candidateRow)}
              </ul>
            )}
          </Panel>
          <p className="text-xs text-muted-foreground">
            这里只显示与本节点相关的配置；保存时整个网络的连接配置一起按版本提交，其他节点的配置保持不变。
            <Link href={routes.connections} className="ml-1 underline underline-offset-4">
              查看全部连接
            </Link>
          </p>
        </>
      ) : (
        <>
          <IdentityPanel
            nodes={nodes}
            revision={latest.revision}
            usage={usage}
            runtimes={runtimes}
          />

          <Panel
            title={<PanelTitle icon={<ArrowRightLeftIcon />} label="主动连接" count={draft.candidates.length} limit={CONNECTION_LIMITS.candidates} />}
            description="箭头是建连方向；建立后两个方向都可复用。"
            actions={addCandidateButton("主动连接节点")}
          >
            {draft.candidates.length === 0 ? (
              <Empty className="py-8">
                <EmptyHeader>
                  <EmptyTitle>还没有主动连接</EmptyTitle>
                  <EmptyDescription>
                    {draft.listeners.length === 0
                      ? "先让某个节点接收连接，其他节点才能连接它。"
                      : "选择发起节点和目标节点，入口地址从目标节点的监听中选择。"}
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : (
              <ul aria-label="拨号候选" className="divide-y">
                {sortedCandidates(draft).map(candidateRow)}
              </ul>
            )}
          </Panel>

          <Panel
            title={<PanelTitle icon={<EarIcon />} label="接收连接的节点" count={draft.listeners.length} limit={CONNECTION_LIMITS.listeners} />}
            description="本机监听地址与对外地址分开填写。"
            actions={addListenerButton("接收其他节点连接", "outline")}
          >
            {draft.listeners.length === 0 ? (
              <Empty className="py-8">
                <EmptyHeader>
                  <EmptyTitle>还没有监听</EmptyTitle>
                  <EmptyDescription>
                    {canAccept
                      ? "空配置是有效的：节点之间不建立任何相邻关系。"
                      : "还没有节点同时发布公钥并声明可监听的承载能力。"}
                  </EmptyDescription>
                </EmptyHeader>
                {canAccept ? <EmptyContent>{addListenerButton("接收其他节点连接", "outline")}</EmptyContent> : null}
              </Empty>
            ) : (
              <ul aria-label="监听器" className="divide-y">
                {sortedListeners(draft).map((listener) => listenerRow(listener, true))}
              </ul>
            )}
          </Panel>
        </>
      )}

      {planIssues.length > 0 || orphanDiagnostics.length > 0 ? (
        <div className="flex flex-col gap-2 rounded-xl border px-4 py-3">
          <ConnectionDiagnosticList label="配置级问题" issues={planIssues} />
          <ConnectionDiagnosticList
            label="已保存配置的其他诊断"
            diagnostics={orphanDiagnostics}
            stale={dirty}
            showItem
          />
        </div>
      ) : null}

      <SaveBar
        dirty={dirty}
        changeSummary={changeSummary}
        issueCount={issues.length}
        baseRevision={base.revision}
        saving={saving}
        disabled={locked}
        save={save}
        onSave={runSave}
        onRevert={() => setConfirm({ kind: "revert" })}
        onReload={requestReload}
      />

      <ListenerSheet
        session={listenerSheet?.session ?? null}
        open={listenerSheet?.open ?? false}
        dirty={working?.kind === "listener"}
        onWorkingChange={trackListenerForm}
        onClose={closeListenerSheet}
        onApply={applyListener}
        onDelete={() => {
          const listener = draft.listeners.find((existing) => existing.key === editingListener?.key)
          closeListenerSheet()
          if (listener) requestRemoveListener(listener)
        }}
        others={draft.listeners.filter((listener) => listener.key !== editingListener?.key)}
        nodes={nodes}
        dialers={editingListener && listenerSheet?.session.mode === "edit" ? dialersOf(editingListener.id.trim()) : []}
        diagnostics={listenerSheetDiagnostics}
        diagnosticsStale={
          editingListener ? listenerChange(editingListener, base.configuration) !== "unchanged" : false
        }
        disabled={locked}
      />

      <CandidateSheet
        session={candidateSheet?.session ?? null}
        open={candidateSheet?.open ?? false}
        dirty={working?.kind === "candidate"}
        onWorkingChange={trackCandidateForm}
        onClose={closeCandidateSheet}
        onApply={applyCandidate}
        onDelete={() => {
          if (editingCandidate) removeCandidate(editingCandidate.key)
          closeCandidateSheet()
        }}
        draft={draft}
        nodes={nodes}
        diagnostics={candidateSheetDiagnostics}
        diagnosticsStale={
          editingCandidate
            ? candidateChange(fromCandidateForm(editingCandidate), base.configuration) !== "unchanged"
            : false
        }
        disabled={locked}
      />

      <AlertDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open) setConfirm(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirm?.kind === "revert"
                ? "放弃草稿中的修改？"
                : confirm?.kind === "delete-listener"
                  ? "删除这个监听？"
                  : "放弃草稿并重新读取？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirm?.kind === "revert"
                ? `草稿恢复为已保存的 r${base.revision} 连接配置，编辑基础不变。`
                : confirm?.kind === "delete-listener"
                  ? `草稿中有 ${confirm.references} 个主动连接使用它。它们不会被自动删除，会保留并标记为需要修复。`
                  : "未保存的修改会被丢弃，替换为 controller 上的最新连接配置与节点身份。"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() => {
                if (confirm?.kind === "revert") revertDraft()
                else if (confirm?.kind === "delete-listener") removeListener(confirm.key)
                else runReload()
              }}
            >
              {confirm?.kind === "revert"
                ? "放弃修改"
                : confirm?.kind === "delete-listener"
                  ? "删除监听"
                  : "放弃草稿并重新读取"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function listeningNodesExcept(draft: ConnectionsDraft, nodeId: string): boolean {
  return draft.listeners.some((listener) => listener.nodeId !== nodeId && listener.nodeId !== "")
}

const byText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0)

/** Listeners grouped by owning node, in draft order within a node. */
function sortedListeners(draft: ConnectionsDraft): ListenerDraft[] {
  return [...draft.listeners].sort((a, b) => byText(a.nodeId, b.nodeId))
}

/** Candidates grouped by dialer, then primary-first within a target. */
function sortedCandidates(draft: ConnectionsDraft): CandidateDraft[] {
  const acceptor = (candidate: CandidateDraft) =>
    draft.listeners.find((listener) => listener.id.trim() === candidate.listenerId)?.nodeId ?? ""
  const rank = (candidate: CandidateDraft) => dialRole(candidate, draft)?.rank ?? 0
  return [...draft.candidates].sort(
    (a, b) =>
      byText(a.dialerNodeId, b.dialerNodeId) || byText(acceptor(a), acceptor(b)) || rank(a) - rank(b)
  )
}

function PanelTitle({
  icon,
  label,
  count,
  limit,
}: {
  icon: ReactNode
  label: string
  count: number
  limit?: number
}) {
  return (
    <span className="flex items-center gap-2 [&_svg]:size-4 [&_svg]:text-muted-foreground">
      <span aria-hidden="true" className="flex">
        {icon}
      </span>
      {label}
      <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
        {limit ? `${count}/${limit}` : count}
      </span>
    </span>
  )
}

function StatusStrip({
  base,
  scopeNodeId,
  reloading,
  disabled,
  onReload,
}: {
  base: ConnectionsSnapshot
  scopeNodeId: string | null
  reloading: boolean
  disabled: boolean
  onReload: () => void
}) {
  return (
    <StatusSummary>
      <Metric label="编辑基础">
        <span data-testid="connections-base-revision" className="font-mono tabular-nums">
          r{base.revision}
        </span>
      </Metric>
      <Metric label="已保存">
        {base.configuration.listeners.length} 个监听 · {base.configuration.candidates.length} 个主动连接
      </Metric>
      <Metric label="状态">
        {base.valid ? (
          <Badge variant="outline" className="text-success">有效</Badge>
        ) : (
          <Badge variant="destructive">无效</Badge>
        )}
      </Metric>
      {scopeNodeId ? (
        <span className="text-muted-foreground">全网共用一份配置</span>
      ) : null}
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="ml-auto"
        disabled={disabled}
        onClick={onReload}
      >
        <RefreshCwIcon data-icon="inline-start" className={reloading ? "animate-spin" : undefined} />
        {reloading ? "正在读取" : "重新读取"}
      </Button>
    </StatusSummary>
  )
}

function SaveBar({
  dirty,
  changeSummary,
  issueCount,
  baseRevision,
  saving,
  disabled,
  save,
  onSave,
  onRevert,
  onReload,
}: {
  dirty: boolean
  changeSummary: string[]
  issueCount: number
  baseRevision: number
  saving: boolean
  disabled: boolean
  save: ConnectionsSaveResult | null
  onSave: () => void
  onRevert: () => void
  onReload: () => void
}) {
  return (
    <DraftActionBar visible={dirty || saving || issueCount > 0 || save !== null}>
      <div data-testid="connections-save-status" aria-live="polite">
        {save?.status === "conflict" ? (
          <Alert variant="destructive" className="mb-1">
            <TriangleAlertIcon />
            <AlertTitle>版本冲突，未保存</AlertTitle>
            <AlertDescription className="flex flex-col items-start gap-2">
              <p>{save.message}</p>
              <Button variant="outline" size="sm" disabled={disabled} onClick={onReload}>
                载入最新版本
              </Button>
            </AlertDescription>
          </Alert>
        ) : save?.status === "invalid" || save?.status === "error" ? (
          <p role="alert" className="mb-1 flex items-start gap-1.5 text-xs text-destructive">
            <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
            <span className="break-all">{save.message}</span>
          </p>
        ) : save?.status === "saved" ? (
          <p role="status" className="mb-1 text-xs text-success">
            已保存为 r{save.snapshot.revision}。节点同步后回执为「已保存」；实际连接以运行状态为准。
          </p>
        ) : null}
      </div>
      <div className="draft-action-bar-row">
        <div className="draft-action-bar-summary text-xs">
          <span
            aria-hidden="true"
            className={cn("size-2 shrink-0 rounded-full", dirty ? "bg-warning" : "bg-muted-foreground/30")}
          />
          <span className="min-w-0">
            {dirty ? (
              <>
                <span className="font-medium">未保存的草稿</span>
                {changeSummary.length > 0 ? (
                  <span className="text-muted-foreground"> · {changeSummary.join("，")}</span>
                ) : null}
              </>
            ) : (
              <span className="text-muted-foreground">草稿与已保存的 r{baseRevision} 一致</span>
            )}
            {issueCount > 0 ? (
              <span className="text-destructive"> · {issueCount} 个问题需修复</span>
            ) : null}
          </span>
        </div>
        <div className="draft-action-bar-actions">
          <Button
            className={dirty ? undefined : "invisible"}
            variant="ghost"
            disabled={disabled || !dirty}
            onClick={onRevert}
          >
            <RotateCcwIcon data-icon="inline-start" />
            放弃修改
          </Button>
          <Button
            size="lg"
            disabled={disabled || !dirty || issueCount > 0}
            onClick={onSave}
            title={`条件保存：仅当 controller 仍为 r${baseRevision} 时写入`}
          >
            {saving ? (
              <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" />
            ) : (
              <SaveIcon data-icon="inline-start" />
            )}
            保存连接配置
          </Button>
        </div>
      </div>
    </DraftActionBar>
  )
}
