"use client"

import type { ConnectionNode, ConnectionsSnapshot } from "@meshway/control-client"
import {
  ArrowRightIcon,
  CircleAlertIcon,
  EarIcon,
  FilterIcon,
  HistoryIcon,
  PencilIcon,
  PhoneOutgoingIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SaveIcon,
  TriangleAlertIcon,
  XIcon,
} from "lucide-react"
import { useRouter } from "next/navigation"
import { useEffect, useMemo, useRef, useState, useTransition, type ReactNode } from "react"

import { CandidateSheet } from "@/components/connections/candidate-sheet"
import {
  ConnectionDiagnosticList,
  diagnosticsFor,
} from "@/components/connections/connection-diagnostics"
import { useConnectionsDraftStore, type Owner } from "@/components/connections/draft-store"
import { IdentityPanel } from "@/components/connections/identity-panel"
import { ListenerSheet } from "@/components/connections/listener-sheet"
import { Panel } from "@/components/common/panel"
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
  reloadConnectionsAction,
  saveConnectionsAction,
} from "@/lib/connections-actions"
import {
  candidateChange,
  canServe,
  CONNECTION_LIMITS,
  draftKey,
  endpointLabel,
  fromPlan,
  listenerChange,
  nodeName,
  planKey,
  removedCount,
  resolveConnectionNode,
  slug,
  toPlan,
  transportLabels,
  transports,
  uniqueId,
  validateConnections,
  type CandidateDraft,
  type CandidateSession,
  type ConnectionIssue,
  type ConnectionsDraft,
  type ItemChange,
  type ListenerDraft,
  type ListenerSession,
  type WorkingSheet,
} from "@/lib/connections-model"
import { cn } from "@/lib/utils"

const changeBadge: Record<ItemChange, { label: string; className: string } | null> = {
  new: { label: "新增", className: "border-brand/40 text-brand" },
  modified: { label: "已修改", className: "border-warning/50 text-warning" },
  unchanged: null,
}

type Confirm =
  | { kind: "reload" }
  | { kind: "revert" }
  | { kind: "delete-listener"; key: string; references: number }

/**
 * Connection control intent: who listens where, and who dials which
 * listener. The editing base (`base`) only changes on a successful save or an
 * explicit reload; a fresher page snapshot is reported, never adopted. Saving
 * freezes editing until the controller answers, so a response can never
 * overwrite edits made in the meantime.
 */
export function ConnectionsEditor({
  snapshot,
  focusNodeId,
}: {
  snapshot: ConnectionsSnapshot
  focusNodeId: string | null
}) {
  const drafts = useConnectionsDraftStore()
  const router = useRouter()
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

  // Identity and receipts come from the freshest read; the CAS base does not.
  const latest = snapshot.revision > base.revision ? snapshot : base
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
  const referencesOf = (listenerId: string) =>
    draft.candidates.filter((candidate) => candidate.listenerId === listenerId).length

  const focusRef = focusNodeId ? resolveConnectionNode(nodes, focusNodeId) : null
  const shownListeners = focusNodeId
    ? draft.listeners.filter((listener) => listener.nodeId === focusNodeId)
    : draft.listeners
  const shownCandidates = focusNodeId
    ? draft.candidates.filter(
        (candidate) =>
          candidate.dialerNodeId === focusNodeId ||
          listenerById(candidate.listenerId)?.nodeId === focusNodeId
      )
    : draft.candidates

  const canAccept = nodes.some((entry) => transports.some((t) => canServe(entry, t, "accept")))

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
  const trackListenerForm = (form: ListenerDraft | null) => {
    const session = listenerSheet?.session
    if (!session) return
    const opened = { id: session.id, mode: session.mode, initial: session.initial }
    setWorking(form === null ? null : { kind: "listener", session: opened, form })
  }
  const trackCandidateForm = (form: CandidateDraft | null) => {
    const session = candidateSheet?.session
    if (!session) return
    const opened = { id: session.id, mode: session.mode, initial: session.initial }
    setWorking(form === null ? null : { kind: "candidate", session: opened, form })
  }

  // --- listeners -----------------------------------------------------------

  const newListener = () => {
    const ready = (entry: ConnectionNode) => transports.some((t) => canServe(entry, t, "accept"))
    const owner =
      (focusRef?.kind === "active" && ready(focusRef.entry) ? focusRef.entry : undefined) ??
      nodes.find(ready)
    const transport = (owner && transports.find((t) => canServe(owner, t, "accept"))) ?? "native_udp"
    const key = `new-${nextKey.current++}`
    sessionId.current += 1
    setWorking(null)
    setListenerSheet({
      open: true,
      session: {
        id: sessionId.current,
        mode: "new",
        initial: {
          key,
          id: uniqueId(
            draft.listeners.map((listener) => listener.id),
            `${slug(owner?.node.name ?? "", "listener")}-in`
          ),
          nodeId: owner?.node.id ?? "",
          transport,
          bind: "",
          endpoints: [{ key: `${key}-e0`, host: "", port: "" }],
          credentialRef: "",
        },
      },
    })
  }
  const editListener = (listener: ListenerDraft) => {
    sessionId.current += 1
    setWorking(null)
    setListenerSheet({ open: true, session: { id: sessionId.current, mode: "edit", initial: listener } })
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
    const references = referencesOf(listener.id.trim())
    if (references > 0) setConfirm({ kind: "delete-listener", key: listener.key, references })
    else removeListener(listener.key)
  }

  // --- candidates ----------------------------------------------------------

  const newCandidate = (listenerId = "") => {
    const listener = listenerById(listenerId)
    const [only] = listener?.endpoints.length === 1 ? listener.endpoints : []
    const key = `new-${nextKey.current++}`
    sessionId.current += 1
    setWorking(null)
    setCandidateSheet({
      open: true,
      session: {
        id: sessionId.current,
        mode: "new",
        initial: {
          key,
          id: uniqueId(
            draft.candidates.map((candidate) => candidate.id),
            listenerId === "" ? "dial" : `dial-${listenerId}`.slice(0, 56)
          ),
          dialerNodeId: "",
          listenerId,
          host: only?.host ?? "",
          port: only?.port ?? "",
          priority: "10",
          credentialRef: "",
        },
      },
    })
  }
  const editCandidate = (candidate: CandidateDraft) => {
    sessionId.current += 1
    setWorking(null)
    setCandidateSheet({ open: true, session: { id: sessionId.current, mode: "edit", initial: candidate } })
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
      const result = await saveConnectionsAction(expected, plan)
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
      const result = await reloadConnectionsAction()
      // Superseded, or this editor unmounted: newer work must survive.
      if (generation !== reloadGeneration.current || !drafts.owns(loader)) return
      if (result.status === "error") {
        setReloadError(result.message)
        return
      }
      setReloadError(null)
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
    changedListeners > 0 ? `${changedListeners} 个监听器新增或修改` : null,
    changedCandidates > 0 ? `${changedCandidates} 个候选新增或修改` : null,
    removed.listeners > 0 ? `删除 ${removed.listeners} 个监听器` : null,
    removed.candidates > 0 ? `删除 ${removed.candidates} 个候选` : null,
  ].filter((part) => part !== null)

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

  return (
    <div data-testid="connections-editor" className="flex flex-col gap-4">
      <StatusStrip
        base={base}
        activeCount={nodes.filter((entry) => entry.node.revoked_at === null).length}
        readyCount={nodes.filter((entry) => entry.node.revoked_at === null && entry.public_key !== null).length}
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
              配置引用了已撤销、未发布公钥或不再具备能力的节点等，节点收到的连接意图为空。原配置保留在下方，修复后保存即可。
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
              离开此页前的修改{restored.sheet ? "（包括未应用的表单内容）" : ""}仍保留在本次会话中，编辑基础仍为 r{restored.base.revision}。刷新浏览器、退出登录会丢弃草稿。
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

      {focusNodeId ? (
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <FilterIcon className="size-3.5 text-muted-foreground" />
          <span>
            仅显示与 <span className="font-medium">{nodeName(nodes, focusNodeId)}</span> 相关的监听器和候选
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => router.replace("/connections", { scroll: false })}
          >
            <XIcon data-icon="inline-start" />
            显示全部
          </Button>
        </div>
      ) : null}

      <Panel
        title={
          <span className="flex items-center gap-2">
            <EarIcon className="size-4 text-muted-foreground" aria-hidden="true" />
            监听器
            <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
              {draft.listeners.length}/{CONNECTION_LIMITS.listeners}
            </span>
          </span>
        }
        description="谁在哪里等待连接。本机监听地址与对外地址分开填写。"
        actions={
          <Button
            size="sm"
            disabled={locked || !canAccept || draft.listeners.length >= CONNECTION_LIMITS.listeners}
            onClick={newListener}
          >
            <PlusIcon data-icon="inline-start" />
            添加监听器
          </Button>
        }
      >
        {draft.listeners.length === 0 ? (
          <Empty className="py-10">
            <EmptyHeader>
              <EmptyTitle>还没有监听器</EmptyTitle>
              <EmptyDescription>
                {canAccept
                  ? "先让一个节点在某个载体上监听，再为其他节点添加拨号候选。空配置是有效的：节点之间不建立任何相邻关系。"
                  : "还没有节点同时发布公钥并声明可监听的承载能力。按下方「节点连接身份」中的命令在节点上发布后即可添加。"}
              </EmptyDescription>
            </EmptyHeader>
            {canAccept ? (
              <EmptyContent>
                <Button variant="outline" disabled={locked} onClick={newListener}>
                  <PlusIcon data-icon="inline-start" />
                  添加第一个监听器
                </Button>
              </EmptyContent>
            ) : null}
          </Empty>
        ) : shownListeners.length === 0 ? (
          <p className="px-4 py-6 text-center text-xs text-muted-foreground">这个节点没有监听器。</p>
        ) : (
          <ul aria-label="监听器" className="divide-y">
            {shownListeners.map((listener) => (
              <ListenerRow
                key={listener.key}
                listener={listener}
                nodes={nodes}
                change={listenerChange(listener, base.configuration)}
                issues={issuesFor(listener.key)}
                diagnostics={diagnosticsFor(base.diagnostics, "listeners", listener.id.trim()).length}
                references={referencesOf(listener.id.trim())}
                highlighted={highlight === listener.key}
                disabled={locked}
                onEdit={() => editListener(listener)}
                onDial={() => newCandidate(listener.id.trim())}
                onDelete={() => requestRemoveListener(listener)}
              />
            ))}
          </ul>
        )}
      </Panel>

      <Panel
        title={
          <span className="flex items-center gap-2">
            <PhoneOutgoingIcon className="size-4 text-muted-foreground" aria-hidden="true" />
            拨号候选
            <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
              {draft.candidates.length}/{CONNECTION_LIMITS.candidates}
            </span>
          </span>
        }
        description="谁主动连接哪个监听器。每个候选只授权这两个节点相邻，不授权 IP 流量、中转或出口。"
        actions={
          <Button
            size="sm"
            disabled={
              locked ||
              draft.listeners.length === 0 ||
              draft.candidates.length >= CONNECTION_LIMITS.candidates
            }
            onClick={() => newCandidate()}
          >
            <PlusIcon data-icon="inline-start" />
            添加候选
          </Button>
        }
      >
        {draft.candidates.length === 0 ? (
          <Empty className="py-10">
            <EmptyHeader>
              <EmptyTitle>还没有拨号候选</EmptyTitle>
              <EmptyDescription>
                {draft.listeners.length === 0
                  ? "先添加监听器。"
                  : "选择一个监听器和发起节点。例如 SHA 主动连接 LAX 的监听器；建立后的通道也可供 LAX 方向的业务复用。"}
              </EmptyDescription>
            </EmptyHeader>
            {draft.listeners.length > 0 ? (
              <EmptyContent>
                <Button variant="outline" disabled={locked} onClick={() => newCandidate()}>
                  <PlusIcon data-icon="inline-start" />
                  添加第一个候选
                </Button>
              </EmptyContent>
            ) : null}
          </Empty>
        ) : shownCandidates.length === 0 ? (
          <p className="px-4 py-6 text-center text-xs text-muted-foreground">这个节点没有相关的拨号候选。</p>
        ) : (
          <>
            <p className="border-b px-4 py-2 text-[0.6875rem] text-muted-foreground">
              箭头表示连接发起方向，数值越小越优先；业务路径不由箭头推导。
            </p>
            <ul aria-label="拨号候选" className="divide-y">
              {shownCandidates.map((candidate) => (
                <CandidateRow
                  key={candidate.key}
                  candidate={candidate}
                  listener={listenerById(candidate.listenerId)}
                  nodes={nodes}
                  change={candidateChange(candidate, base.configuration)}
                  issues={issuesFor(candidate.key)}
                  diagnostics={diagnosticsFor(base.diagnostics, "candidates", candidate.id.trim()).length}
                  highlighted={highlight === candidate.key}
                  disabled={locked}
                  onEdit={() => editCandidate(candidate)}
                  onDelete={() => removeCandidate(candidate.key)}
                />
              ))}
            </ul>
          </>
        )}
      </Panel>

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

      <IdentityPanel nodes={nodes} revision={latest.revision} usage={usage} focusNodeId={focusNodeId} />

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
        referencedBy={editingListener ? referencesOf(editingListener.id.trim()) : 0}
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
        others={draft.candidates.filter((candidate) => candidate.key !== editingCandidate?.key)}
        listeners={draft.listeners}
        nodes={nodes}
        diagnostics={candidateSheetDiagnostics}
        diagnosticsStale={
          editingCandidate ? candidateChange(editingCandidate, base.configuration) !== "unchanged" : false
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
                  ? "删除这个监听器？"
                  : "放弃草稿并重新读取？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirm?.kind === "revert"
                ? `草稿恢复为已保存的 r${base.revision} 连接配置，编辑基础不变。`
                : confirm?.kind === "delete-listener"
                  ? `草稿中有 ${confirm.references} 个拨号候选引用它。候选不会被自动删除，会保留并标记为需要修复。`
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
                  ? "删除监听器"
                  : "放弃草稿并重新读取"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function useScrollIntoView(highlighted: boolean) {
  const ref = useRef<HTMLLIElement>(null)
  useEffect(() => {
    if (highlighted) ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" })
  }, [highlighted])
  return ref
}

function RowBadges({
  change,
  issues,
  diagnostics,
}: {
  change: ItemChange
  issues: number
  diagnostics: number
}) {
  const badge = changeBadge[change]
  // Saved-source diagnostics only describe an unchanged item.
  const problems = issues + (change === "unchanged" ? diagnostics : 0)
  return (
    <>
      {badge ? (
        <Badge variant="outline" className={badge.className}>
          {badge.label}
        </Badge>
      ) : null}
      {problems > 0 ? (
        <Badge variant="destructive">
          <CircleAlertIcon data-icon="inline-start" />
          {problems} 个问题
        </Badge>
      ) : null}
    </>
  )
}

function ListenerRow({
  listener,
  nodes,
  change,
  issues,
  diagnostics,
  references,
  highlighted,
  disabled,
  onEdit,
  onDial,
  onDelete,
}: {
  listener: ListenerDraft
  nodes: ConnectionNode[]
  change: ItemChange
  issues: ConnectionIssue[]
  diagnostics: number
  references: number
  highlighted: boolean
  disabled: boolean
  onEdit: () => void
  onDial: () => void
  onDelete: () => void
}) {
  const ref = useScrollIntoView(highlighted)
  const label = listener.id.trim() || "（空 ID）"
  const owner = resolveConnectionNode(nodes, listener.nodeId)
  const problems = issues.length + (change === "unchanged" ? diagnostics : 0)
  return (
    <li
      ref={ref}
      data-testid="listener-row"
      data-listener-id={listener.id.trim()}
      aria-label={`监听器 ${label}`}
      className={cn(
        "group/row relative flex items-start gap-3 px-3 py-3 transition-colors hover:bg-muted/30 sm:px-4",
        highlighted && "bg-brand/[0.06] ring-1 ring-brand/40 ring-inset",
        problems > 0 && "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-destructive"
      )}
    >
      <button
        type="button"
        onClick={onEdit}
        disabled={disabled}
        aria-label={`编辑监听器 ${label}`}
        className="flex min-w-0 flex-1 flex-col gap-1.5 text-left focus-visible:outline-none disabled:cursor-not-allowed"
      >
        <span className="flex flex-wrap items-center gap-2">
          <span className="truncate font-mono text-sm font-medium group-hover/row:underline">{label}</span>
          <Badge variant="secondary" className="font-normal">
            {transportLabels[listener.transport]}
          </Badge>
          <RowBadges change={change} issues={issues.length} diagnostics={diagnostics} />
        </span>
        <span className="grid gap-x-4 gap-y-1 text-xs sm:grid-cols-[minmax(0,11rem)_minmax(0,12rem)_minmax(0,1fr)]">
          <Field label="监听节点">
            <span className={cn(owner.kind !== "active" && "text-destructive")}>
              {nodeName(nodes, listener.nodeId)}
            </span>
          </Field>
          <Field label="本机监听">
            <code className="font-mono">{listener.bind.trim() || "—"}</code>
          </Field>
          <Field label="对外地址">
            <span className="flex min-w-0 flex-wrap gap-x-2 font-mono">
              {listener.endpoints.length === 0
                ? "—"
                : listener.endpoints.map((endpoint) => (
                    <span key={endpoint.key}>{endpointLabel(endpoint.host, endpoint.port)}</span>
                  ))}
            </span>
          </Field>
        </span>
        <span className="text-[0.6875rem] text-muted-foreground">
          {references > 0 ? `${references} 个拨号候选` : "还没有节点拨号到这里"}
          {listener.credentialRef.trim() !== "" ? (
            <>
              {" · 配置档 "}
              <code className="font-mono">{listener.credentialRef.trim()}</code>
            </>
          ) : null}
        </span>
      </button>
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={disabled || listener.id.trim() === ""}
          onClick={onDial}
          aria-label={`为监听器 ${label} 添加拨号候选`}
          className="hidden sm:inline-flex"
        >
          <PhoneOutgoingIcon data-icon="inline-start" />
          添加候选
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          onClick={onEdit}
          aria-label={`编辑监听器 ${label} 详情`}
        >
          <PencilIcon />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          onClick={onDelete}
          aria-label={`删除监听器 ${label}`}
          className="text-muted-foreground hover:text-destructive"
        >
          <XIcon />
        </Button>
      </div>
    </li>
  )
}

function CandidateRow({
  candidate,
  listener,
  nodes,
  change,
  issues,
  diagnostics,
  highlighted,
  disabled,
  onEdit,
  onDelete,
}: {
  candidate: CandidateDraft
  listener: ListenerDraft | undefined
  nodes: ConnectionNode[]
  change: ItemChange
  issues: ConnectionIssue[]
  diagnostics: number
  highlighted: boolean
  disabled: boolean
  onEdit: () => void
  onDelete: () => void
}) {
  const ref = useScrollIntoView(highlighted)
  const label = candidate.id.trim() || "（空 ID）"
  const dialer = resolveConnectionNode(nodes, candidate.dialerNodeId)
  const acceptor = listener ? resolveConnectionNode(nodes, listener.nodeId) : null
  const problems = issues.length + (change === "unchanged" ? diagnostics : 0)
  return (
    <li
      ref={ref}
      data-testid="connection-candidate-row"
      data-candidate-id={candidate.id.trim()}
      aria-label={`拨号候选 ${label}`}
      className={cn(
        "group/row relative flex items-start gap-3 px-3 py-3 transition-colors hover:bg-muted/30 sm:px-4",
        highlighted && "bg-brand/[0.06] ring-1 ring-brand/40 ring-inset",
        problems > 0 && "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-destructive"
      )}
    >
      <span
        className="mt-0.5 flex h-6 min-w-8 shrink-0 items-center justify-center rounded-md border bg-background px-1 font-mono text-[0.6875rem] text-muted-foreground tabular-nums"
        title="优先级：数值越小越优先"
      >
        {candidate.priority.trim() || "—"}
      </span>
      <button
        type="button"
        onClick={onEdit}
        disabled={disabled}
        aria-label={`编辑拨号候选 ${label}`}
        className="flex min-w-0 flex-1 flex-col gap-1.5 text-left focus-visible:outline-none disabled:cursor-not-allowed"
      >
        <span className="flex flex-wrap items-center gap-2">
          <span className="truncate font-mono text-sm font-medium group-hover/row:underline">{label}</span>
          {listener ? (
            <Badge variant="secondary" className="font-normal">
              {transportLabels[listener.transport]}
            </Badge>
          ) : null}
          <RowBadges change={change} issues={issues.length} diagnostics={diagnostics} />
        </span>
        <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
          <span className="text-muted-foreground">发起</span>
          <span className={cn("font-medium", dialer.kind !== "active" && "text-destructive")}>
            {nodeName(nodes, candidate.dialerNodeId)}
          </span>
          <ArrowRightIcon className="size-3.5 text-muted-foreground" aria-label="连接发起方向" />
          <span className="text-muted-foreground">监听</span>
          <span className={cn("font-medium", (!acceptor || acceptor.kind !== "active") && "text-destructive")}>
            {listener ? nodeName(nodes, listener.nodeId) : `未知监听器 ${candidate.listenerId || "—"}`}
          </span>
          <code className="font-mono text-muted-foreground">
            {candidate.listenerId}
            {candidate.host.trim() !== "" ? ` @ ${endpointLabel(candidate.host, candidate.port)}` : ""}
          </code>
        </span>
      </button>
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          onClick={onEdit}
          aria-label={`编辑拨号候选 ${label} 详情`}
        >
          <PencilIcon />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          onClick={onDelete}
          aria-label={`删除拨号候选 ${label}`}
          className="text-muted-foreground hover:text-destructive"
        >
          <XIcon />
        </Button>
      </div>
    </li>
  )
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex min-w-0 items-baseline gap-1.5">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 truncate">{children}</span>
    </span>
  )
}

function StatusStrip({
  base,
  activeCount,
  readyCount,
  reloading,
  disabled,
  onReload,
}: {
  base: ConnectionsSnapshot
  activeCount: number
  readyCount: number
  reloading: boolean
  disabled: boolean
  onReload: () => void
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-xl border bg-card px-4 py-2.5 text-xs">
      <Metric label="编辑基础">
        <span data-testid="connections-base-revision" className="font-mono tabular-nums">
          r{base.revision}
        </span>
      </Metric>
      <Metric label="已保存">
        {base.configuration.listeners.length} 监听器 · {base.configuration.candidates.length} 候选
      </Metric>
      <Metric label="状态">
        {base.valid ? (
          <Badge variant="outline" className="text-success">有效</Badge>
        ) : (
          <Badge variant="destructive">无效</Badge>
        )}
      </Metric>
      <Metric label="已发布公钥">
        <span className="tabular-nums">
          {readyCount}/{activeCount}
        </span>
      </Metric>
      <Badge
        variant="outline"
        className="font-normal text-muted-foreground"
        title="保存的是控制意图；节点回执只表示 agent 已校验并保存，不代表连接已建立或已生效。"
      >
        控制意图 / 已保存回执
      </Badge>
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
    </div>
  )
}

function Metric({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex items-center gap-1.5">
      <span className="text-muted-foreground">{label}</span>
      {children}
    </span>
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
    <div className="sticky bottom-3 z-10 flex flex-col gap-2 rounded-xl border bg-background/95 px-4 py-3 shadow-lg backdrop-blur supports-backdrop-filter:bg-background/80">
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
            已保存为 r{save.snapshot.revision}。节点同步后回执为「已保存」，不代表连接已建立。
          </p>
        ) : null}
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-2 text-xs">
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
        <div className="flex items-center gap-2">
          {dirty ? (
            <Button variant="ghost" disabled={disabled} onClick={onRevert}>
              <RotateCcwIcon data-icon="inline-start" />
              放弃修改
            </Button>
          ) : null}
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
    </div>
  )
}
