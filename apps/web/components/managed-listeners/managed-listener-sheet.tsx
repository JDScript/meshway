"use client"

import type { ConnectionNode, ConnectionsSnapshot, ManagedListenersSnapshot } from "@meshway/control-client"
import { CircleAlertIcon, InfoIcon, RefreshCwIcon, TriangleAlertIcon } from "lucide-react"
import { useEffect, useMemo, useRef, useState, useTransition, type ReactNode } from "react"

import { useNetworkId } from "@/components/network/network-workspace"
import { ManagedListenerFields } from "@/components/managed-listeners/managed-listener-fields"
import { useManagedListenerDrafts } from "@/components/managed-listeners/draft-store"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Sheet, SheetContent, SheetDescription, SheetFooter, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Spinner } from "@/components/ui/spinner"
import type { ManagedListenerReadResult, ManagedListenerSaveResult } from "@/lib/action-state"
import { nodeName } from "@/lib/connections-model"
import { readManagedListenersAction, saveManagedListenerAction } from "@/lib/managed-listener-actions"
import { draftChanged, type ManagedListenerDraft } from "@/lib/managed-listener-draft-registry"
import { formatClock } from "@/lib/time"
import {
  allowedPresets,
  changedFields,
  changeImpact,
  changePreset,
  followSuggestions,
  managedById,
  referencingCandidates,
  takenListenerIds,
  toSettings,
  validateManagedForm,
  type ManagedListenerForm,
  type ManagedPreset,
} from "@/lib/managed-listener-model"

type Outcome =
  | { status: "conflict" | "invalid" | "not_found" | "error"; message: string }
  | { status: "rebased"; changed: string[] }
  | null

/**
 * Create or edit one managed listener of a node. The form, its CAS base and
 * any failure stay in the network's draft store: switching tabs, navigating
 * away or a background read never replaces them. Only a successful save
 * clears the draft; a conflict offers an explicit rebase.
 */
export function ManagedListenerSheet({
  session,
  managed,
  cachedAt,
  connections,
  nodes,
  onClose,
  onSaved,
  onRead,
  onReadFailed,
}: {
  /** The draft being edited, or null when closed. */
  session: ManagedListenerDraft | null
  /** Latest managed read on the page (display / rebase source, never the base by itself). */
  managed: ManagedListenersSnapshot
  /** Read time of `managed` when the latest managed read failed (it is a cache), else null. */
  cachedAt: number | null
  connections: ConnectionsSnapshot | null
  nodes: ConnectionNode[]
  onClose: () => void
  onSaved: (result: Extract<ManagedListenerSaveResult, { status: "saved" }>) => void
  onRead: (result: Extract<ManagedListenerReadResult, { status: "loaded" }>) => void
  onReadFailed: (message: string, at: number) => void
}) {
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const drafts = useManagedListenerDrafts()
  const dirty = session !== null && drafts.get(session.key) !== undefined
  const requestClose = () => {
    if (session && drafts.get(session.key)) setConfirmDiscard(true)
    else onClose()
  }
  return (
    <>
      <Sheet
        open={session !== null}
        onOpenChange={(open) => {
          if (!open) requestClose()
        }}
      >
        <SheetContent className="w-full gap-0 data-[side=right]:w-full sm:max-w-xl data-[side=right]:sm:max-w-xl">
          {session ? (
            <SheetForm
              key={session.key}
              session={session}
              managed={managed}
              cachedAt={cachedAt}
              connections={connections}
              nodes={nodes}
              dirty={dirty}
              onCancel={requestClose}
              onSaved={onSaved}
              onRead={onRead}
              onReadFailed={onReadFailed}
              onDone={onClose}
            />
          ) : null}
        </SheetContent>
      </Sheet>
      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>放弃对这个监听的修改？</AlertDialogTitle>
            <AlertDialogDescription>表单中的修改尚未保存；已保存的监听不受影响。「稍后继续」会在这个网络中保留表单，直到保存、放弃或退出登录。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <Button
              variant="outline"
              onClick={() => {
                setConfirmDiscard(false)
                onClose()
              }}
            >
              稍后继续
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                setConfirmDiscard(false)
                if (session) drafts.remove(session.key)
                onClose()
              }}
            >
              放弃修改
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function SheetForm({
  session,
  managed,
  cachedAt,
  connections,
  nodes,
  dirty,
  onCancel,
  onSaved,
  onRead,
  onReadFailed,
  onDone,
}: {
  session: ManagedListenerDraft
  managed: ManagedListenersSnapshot
  cachedAt: number | null
  connections: ConnectionsSnapshot | null
  nodes: ConnectionNode[]
  dirty: boolean
  onCancel: () => void
  onSaved: (result: Extract<ManagedListenerSaveResult, { status: "saved" }>) => void
  onRead: (result: Extract<ManagedListenerReadResult, { status: "loaded" }>) => void
  onReadFailed: (message: string, at: number) => void
  onDone: () => void
}) {
  const networkId = useNetworkId()
  const drafts = useManagedListenerDrafts()
  const [draft, setDraft] = useState<ManagedListenerDraft>(() => drafts.get(session.key) ?? session)
  const [attempted, setAttempted] = useState(draft !== session)
  const [outcome, setOutcome] = useState<Outcome>(null)
  const [saving, startSave] = useTransition()
  const [reading, startRead] = useTransition()
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  const creating = draft.kind === "create"
  const nodeId = draft.nodeId ?? ""
  const taken = useMemo(() => takenListenerIds(managed, connections), [managed, connections])
  const issues = useMemo(() => validateManagedForm(draft.form, { creating, takenIds: taken }), [draft.form, creating, taken])
  const visibleIssues = attempted ? issues : []
  const candidates = useMemo(
    () => (draft.original ? referencingCandidates(connections, draft.original.id, draft.form.endpoints) : []),
    [connections, draft.original, draft.form.endpoints]
  )
  const impact = changeImpact(draft.original, draft.form, candidates)
  // Against the saved version currently adopted as the CAS base, not the one first opened.
  const unchanged = !creating && !draftChanged(draft)
  const saved = draft.original ? managedById(managed, draft.original.id) : undefined
  const savedMoved =
    draft.original !== null && saved !== undefined && JSON.stringify(saved.settings) !== JSON.stringify(draft.original)
  const behind = draft.baseRevision !== null && managed.revision > draft.baseRevision

  // The store keeps a draft only while it differs from its adopted baseline.
  const keep = (next: ManagedListenerDraft) => {
    if (draftChanged(next)) drafts.put({ ...next, open: true })
    else drafts.remove(next.key)
  }
  const update = (next: ManagedListenerDraft) => {
    setDraft(next)
    keep(next)
  }
  // Typing an ID stops it following the preset's suggestion.
  const setForm = (form: ManagedListenerForm) =>
    update({ ...draft, form: creating ? followSuggestions(draft.form, form) : form, idFollows: draft.idFollows && form.id === draft.form.id })
  const setPreset = (preset: ManagedPreset) => {
    const form = changePreset(draft.form, preset, { idFollows: creating && draft.idFollows, taken, prefix: idPrefix(nodes, nodeId) })
    update({ ...draft, form })
  }

  const save = () => {
    setAttempted(true)
    setOutcome(null)
    if (issues.length > 0 || draft.baseRevision === null || unchanged) return
    const sent: ManagedListenerDraft = { ...draft, open: true }
    drafts.put(sent)
    setDraft(sent)
    const settings = toSettings(sent.form, nodeId)
    startSave(async () => {
      const result = await saveManagedListenerAction(networkId, sent.baseRevision ?? 0, settings)
      if (result.status === "saved") {
        drafts.settle(sent.key, sent)
        onSaved(result)
        if (mounted.current) onDone()
        return
      }
      if (mounted.current) setOutcome(result)
    })
  }

  const rebase = () => {
    startRead(async () => {
      const result = await readManagedListenersAction(networkId)
      if (result.status !== "loaded") {
        onReadFailed(result.message, result.fetchedAt)
        if (mounted.current) setOutcome({ status: "error", message: result.message })
        return
      }
      onRead(result)
      if (!mounted.current) return
      const latest = draft.original ? managedById(result.managed, draft.original.id) : undefined
      if (draft.original && !latest) {
        setOutcome({ status: "not_found", message: "这个监听已不在 controller 中。" })
        return
      }
      const changed = draft.original && latest ? changedFields(draft.original, latest.settings) : []
      const next: ManagedListenerDraft = {
        ...draft,
        baseRevision: result.managed.revision,
        original: latest?.settings ?? draft.original,
      }
      // The typed form stays; only its baseline and CAS base move.
      setDraft(next)
      keep(next)
      setOutcome({ status: "rebased", changed })
    })
  }

  const busy = saving || reading
  const title = creating ? "添加监听" : `编辑监听 ${draft.original?.id ?? ""}`

  return (
    <form
      className="flex min-h-0 flex-1 flex-col"
      aria-busy={busy || undefined}
      onSubmit={(event) => {
        event.preventDefault()
        save()
      }}
    >
      <SheetHeader className="gap-1 border-b p-4 pr-12">
        <SheetTitle className="text-base">{title}</SheetTitle>
        <SheetDescription>
          {nodeName(nodes, nodeId)} · 平台托管 · 基于 r{draft.baseRevision ?? "—"}
        </SheetDescription>
      </SheetHeader>

      <div className="flex-1 overflow-y-auto">
        {cachedAt !== null ? (
          <Notice tone="warning" icon={<TriangleAlertIcon />}>
            <span data-testid="managed-listener-cached">
              托管监听最近一次读取失败；页面显示 {formatClock(cachedAt)} 的缓存（r{managed.revision}）。表单基于 r{draft.baseRevision ?? "—"}，保存时由 controller 做 CAS 校验。
            </span>
          </Notice>
        ) : null}
        {behind || savedMoved ? (
          <Notice tone="warning" icon={<TriangleAlertIcon />}>
            <span>
              {savedMoved ? "这个监听在你编辑后已被修改。" : `网络配置已更新到 r${managed.revision}，保存可能冲突。`}
            </span>
            <Button type="button" variant="outline" size="sm" disabled={busy} onClick={rebase}>
              <RefreshCwIcon data-icon="inline-start" className={reading ? "animate-spin" : undefined} />
              以最新版本为基础
            </Button>
          </Notice>
        ) : null}
        <ManagedListenerFields
          form={draft.form}
          onChange={(form) => setForm(form)}
          onPresetChange={setPreset}
          creating={creating}
          allowed={allowedPresets(draft.original)}
          issues={visibleIssues}
          disabled={busy}
        />
        <ImpactSection
          creating={creating}
          candidates={candidates}
          impact={impact}
          nodes={nodes}
          connectionsKnown={connections !== null}
        />
      </div>

      <div aria-live="polite" className="empty:hidden">
        {outcome ? <OutcomeNotice outcome={outcome} busy={busy} onRebase={rebase} /> : null}
        {attempted && issues.length > 0 ? (
          <Notice tone="danger" icon={<CircleAlertIcon />}>
            <span>还有 {issues.length} 处需要修改，未保存。</span>
          </Notice>
        ) : null}
      </div>

      <SheetFooter className="flex-row items-center gap-2 border-t p-3">
        <span className="min-w-0 truncate text-xs text-muted-foreground">
          {dirty ? "未保存的修改" : unchanged ? "没有修改" : null}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <Button type="button" variant="outline" onClick={onCancel} disabled={saving}>
            取消
          </Button>
          <Button type="submit" disabled={busy || unchanged}>
            {saving ? <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" /> : null}
            {creating ? "创建监听" : "保存监听"}
          </Button>
        </div>
      </SheetFooter>
    </form>
  )
}

function idPrefix(nodes: ConnectionNode[], nodeId: string): string {
  const name = nodes.find((entry) => entry.node.id === nodeId)?.node.name ?? ""
  return name
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 20)
}

export { idPrefix as managedIdPrefix }

function ImpactSection({
  creating,
  candidates,
  impact,
  nodes,
  connectionsKnown,
}: {
  creating: boolean
  candidates: ReturnType<typeof referencingCandidates>
  impact: ReturnType<typeof changeImpact>
  nodes: ConnectionNode[]
  connectionsKnown: boolean
}) {
  if (creating) {
    return (
      <p className="flex items-start gap-1.5 border-t px-4 py-3 text-xs text-muted-foreground">
        <InfoIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
        创建监听只保存入口；其他节点要拨入，仍需在「高级手工连接配置」中添加拨号候选。
      </p>
    )
  }
  return (
    <section data-testid="managed-listener-impact" className="flex flex-col gap-2 border-t px-4 py-4 text-xs">
      <h3 className="font-semibold tracking-wide text-muted-foreground uppercase">影响</h3>
      {!connectionsKnown ? (
        <p className="text-warning">连接配置读取失败，无法列出引用这个监听的拨号候选。</p>
      ) : candidates.length === 0 ? (
        <p className="text-muted-foreground">没有拨号候选引用这个监听。</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {candidates.map((candidate) => (
            <li key={candidate.id} className="flex flex-wrap items-center gap-x-2">
              <span className="font-medium">{nodeName(nodes, candidate.dialerNodeId)}</span>
              <span className="text-muted-foreground">拨向</span>
              <code className="font-mono">{candidate.endpoint}</code>
              <code className="font-mono text-muted-foreground">{candidate.id}</code>
            </li>
          ))}
        </ul>
      )}
      {impact.disablesReferenced > 0 ? (
        <p role="status" className="flex items-start gap-1.5 text-warning">
          <TriangleAlertIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          停用后这 {impact.disablesReferenced} 个拨号候选仍保留，但连接配置会变为无效、相关通道撤回，直到重新启用或在高级入口中修复。
        </p>
      ) : null}
      {impact.reenables ? <p className="text-muted-foreground">重新启用会恢复这些拨号候选。</p> : null}
      {impact.orphanedCandidates.length > 0 ? (
        <p role="status" className="flex items-start gap-1.5 text-warning">
          <TriangleAlertIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          {impact.orphanedCandidates.map((candidate) => candidate.endpoint).join("、")} 不在对外地址中：对应候选会失效，需在高级入口中改到新地址。
        </p>
      ) : null}
      {impact.rotatesMaterial ? (
        <p role="status" className="flex items-start gap-1.5 text-warning">
          <TriangleAlertIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          TLS 名称变化：平台会签发新证书与 UUID，旧代际在新版本下发后撤回。
        </p>
      ) : null}
    </section>
  )
}

function OutcomeNotice({ outcome, busy, onRebase }: { outcome: NonNullable<Outcome>; busy: boolean; onRebase: () => void }) {
  if (outcome.status === "rebased") {
    return (
      <Notice tone="info" icon={<InfoIcon />}>
        <span data-testid="managed-listener-rebased">
          已改为基于最新版本
          {outcome.changed.length > 0 ? `；已保存的监听在此期间改了：${outcome.changed.join("、")}。你的表单保持不变，确认后再保存。` : "；你的表单保持不变。"}
        </span>
      </Notice>
    )
  }
  return (
    <Notice tone={outcome.status === "conflict" ? "warning" : "danger"} icon={outcome.status === "conflict" ? <TriangleAlertIcon /> : <CircleAlertIcon />}>
      <span data-testid="managed-listener-outcome" data-status={outcome.status} role="alert">
        {outcome.message}
      </span>
      {outcome.status === "conflict" ? (
        <Button type="button" variant="outline" size="sm" disabled={busy} onClick={onRebase}>
          <RefreshCwIcon data-icon="inline-start" />
          读取最新版本
        </Button>
      ) : null}
    </Notice>
  )
}

function Notice({ tone, icon, children }: { tone: "warning" | "danger" | "info"; icon: ReactNode; children: ReactNode }) {
  const color = tone === "danger" ? "border-destructive/30 bg-destructive/5 [&>svg]:text-destructive" : tone === "warning" ? "border-warning/40 bg-warning/5 [&>svg]:text-warning" : "border-info/30 bg-info/5 [&>svg]:text-info"
  return (
    <div className={`m-3 flex flex-wrap items-center gap-2 rounded-lg border px-3 py-2 text-xs [&>svg]:size-3.5 [&>svg]:shrink-0 ${color}`}>
      {icon}
      {children}
    </div>
  )
}
