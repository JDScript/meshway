"use client"

import type {
  ConnectionNode,
  MeshAccessGrant,
  MeshAccessSnapshot,
} from "@meshway/control-client"
import {
  ArrowLeftRightIcon,
  ArrowUpRightIcon,
  CircleAlertIcon,
  EllipsisIcon,
  HistoryIcon,
  PencilIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SaveIcon,
  ShieldCheckIcon,
  Trash2Icon,
  TriangleAlertIcon,
} from "lucide-react"
import Link from "next/link"
import { useEffect, useMemo, useRef, useState, useTransition, type ReactNode } from "react"

import { useAccessDraftStore, type Owner } from "@/components/access/draft-store"
import { GrantBuilder } from "@/components/access/grant-builder"
import { GrantPath } from "@/components/access/grant-path"
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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Spinner } from "@/components/ui/spinner"
import { useToastManager } from "@/components/ui/toast"
import { saveMeshAccessAction, reloadMeshAccessAction } from "@/lib/access-actions"
import {
  ACCESS_LIMITS,
  accessFieldLabel,
  accessKey,
  copyAccess,
  diagnosticsForGrant,
  grantChange,
  hasReverse,
  issuesForGrant,
  newGrantForm,
  newGrantId,
  readyNodes,
  removedGrantCount,
  reversePath,
  sameAccess,
  settleSave,
  toGrantForm,
  validateAccess,
  validatePath,
  type GrantChange,
  type GrantForm,
  type GrantSession,
  type WorkingGrant,
} from "@/lib/access-model"
import type { AccessSaveResult } from "@/lib/action-state"
import { docsUrl } from "@/lib/config"
import { cn } from "@/lib/utils"

const changeBadge: Record<GrantChange, { label: string; className: string } | null> = {
  new: { label: "新增", className: "border-brand/40 text-brand" },
  modified: { label: "已修改", className: "border-warning/50 text-warning" },
  unchanged: null,
}

/**
 * The editing base (`base`) only ever changes on a successful save or an
 * explicit reload. A fresher page render may reveal a newer controller
 * revision, which is reported but never adopted silently: the conditional
 * save always sends the revision the draft was loaded from.
 */
export function AccessEditor({
  snapshot,
  inventory: initialInventory,
}: {
  snapshot: MeshAccessSnapshot
  inventory: ConnectionNode[]
}) {
  const drafts = useAccessDraftStore()
  const [restored] = useState(() => drafts.read())
  const [restoredNotice, setRestoredNotice] = useState(restored !== null)
  const [base, setBase] = useState(restored?.base ?? snapshot)
  const [inventory, setInventory] = useState(restored?.inventory ?? initialInventory)
  const [draft, setDraft] = useState(() => restored?.draft ?? copyAccess(snapshot.policy))
  const [save, setSave] = useState<AccessSaveResult | null>(null)
  const [reloadError, setReloadError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<"reload" | "revert" | null>(null)
  const [working, setWorking] = useState<WorkingGrant | null>(restored?.builder ?? null)
  const [builder, setBuilder] = useState<{ session: GrantSession; open: boolean } | null>(() =>
    restored?.builder
      ? { open: true, session: { ...restored.builder.session, working: restored.builder.form } }
      : null
  )
  const [highlight, setHighlight] = useState<string | null>(null)
  const [saving, startSave] = useTransition()
  const [reloading, startReload] = useTransition()
  const generation = useRef(0)
  const undoToasts = useRef<string[]>([])
  const sessionId = useRef(restored?.nextSession ?? 0)
  const draftRef = useRef(draft)
  const owner = useRef<Owner | null>(null)
  const reloadGeneration = useRef(0)
  const toast = useToastManager()

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

  const dirty = !sameAccess(draft, base.policy)
  const busy = saving || reloading
  const issues = useMemo(() => validateAccess(draft, inventory), [draft, inventory])
  const behind = snapshot.revision > base.revision

  useEffect(() => {
    if (dirty || working !== null) {
      drafts.write(owner.current, {
        base,
        inventory,
        draft,
        builder: working,
        nextSession: sessionId.current,
      })
    } else {
      drafts.clear(owner.current)
    }
  }, [dirty, working, base, inventory, draft, drafts])

  const updateDraft = (update: (current: typeof draft) => typeof draft) => {
    setDraft(update)
    setSave(null)
  }

  const newGeneration = () => {
    generation.current += 1
    for (const id of undoToasts.current) toast.close(id)
    undoToasts.current = []
    setRestoredNotice(false)
  }

  const openBuilder = (mode: GrantSession["mode"], initial: GrantForm) => {
    sessionId.current += 1
    setWorking(null)
    setBuilder({ session: { id: sessionId.current, mode, initial }, open: true })
  }
  const closeBuilder = () => {
    setWorking(null)
    setBuilder((current) => (current ? { ...current, open: false } : null))
  }
  const trackBuilderForm = (form: GrantForm | null) => {
    const session = builder?.session
    if (!session) return
    const opened = { id: session.id, mode: session.mode, initial: session.initial }
    setWorking(form === null ? null : { session: opened, form })
  }

  const newGrant = () => openBuilder("new", newGrantForm(newGrantId()))
  const editGrant = (grant: MeshAccessGrant) => openBuilder("edit", toGrantForm(grant))

  const applyGrants = (grants: MeshAccessGrant[]) => {
    const [main] = grants
    if (!main) return
    updateDraft((current) => {
      const existing = current.grants.findIndex((grant) => grant.id === main.id)
      const next = [...current.grants]
      if (existing >= 0) next.splice(existing, 1, ...grants)
      else next.push(...grants)
      return { grants: next }
    })
    setHighlight(main.id)
    closeBuilder()
  }

  const addReverse = (grant: MeshAccessGrant) => {
    if (hasReverse(grant, draft.grants) || draft.grants.length >= ACCESS_LIMITS.grants) return
    const reverse = { id: newGrantId(), path: reversePath(grant.path) }
    updateDraft((current) => {
      const index = current.grants.findIndex((candidate) => candidate.id === grant.id)
      if (index < 0 || hasReverse(grant, current.grants)) return current
      const next = [...current.grants]
      next.splice(index + 1, 0, reverse)
      return { grants: next }
    })
    setHighlight(reverse.id)
  }

  const deleteGrant = (id: string) => {
    const index = draft.grants.findIndex((grant) => grant.id === id)
    const grant = draft.grants[index]
    if (!grant) return
    updateDraft((current) => ({ grants: current.grants.filter((candidate) => candidate.id !== id) }))
    const owned = generation.current
    const toastId = toast.add({
      title: "已从草稿删除一条授权",
      description: "保存前不会影响 controller。",
      actionProps: {
        children: "撤销",
        onClick: () => {
          undoToasts.current = undoToasts.current.filter((existing) => existing !== toastId)
          if (owned !== generation.current) return
          updateDraft((current) => {
            if (owned !== generation.current) return current
            if (current.grants.some((candidate) => candidate.id === id)) return current
            const next = [...current.grants]
            next.splice(Math.min(index, next.length), 0, grant)
            return { grants: next }
          })
        },
      },
    })
    undoToasts.current = [...undoToasts.current, toastId]
  }

  const runSave = () => {
    const expected = base.revision
    const body = draft
    const sentKey = accessKey(draft)
    const saver = owner.current
    startSave(async () => {
      const result = await saveMeshAccessAction(expected, body)
      if (!drafts.owns(saver)) {
        if (result.status === "saved") drafts.adoptOrphanSave(expected, sentKey)
        return
      }
      setSave(result)
      if (result.status === "saved") {
        newGeneration()
        const next = settleSave({ base, draft: draftRef.current }, result, sentKey)
        setBase(next.base)
        setDraft(next.draft)
      }
    })
  }

  const runReload = () => {
    setConfirm(null)
    const requested = ++reloadGeneration.current
    const loader = owner.current
    startReload(async () => {
      const result = await reloadMeshAccessAction()
      if (requested !== reloadGeneration.current || !drafts.owns(loader)) return
      if (result.status === "error") {
        setReloadError(result.message)
        return
      }
      setReloadError(null)
      // The administrator confirmed discarding the draft.
      setWorking(null)
      setBuilder((current) => (current ? { ...current, open: false } : null))
      newGeneration()
      setBase(result.snapshot)
      setInventory(result.inventory)
      setDraft(copyAccess(result.snapshot.policy))
      setSave(null)
      setHighlight(null)
    })
  }
  const requestReload = () => (dirty ? setConfirm("reload") : runReload())
  const revertDraft = () => {
    setConfirm(null)
    newGeneration()
    setDraft(copyAccess(base.policy))
    setSave(null)
  }

  const changed = draft.grants.filter((grant) => grantChange(grant, base.policy) !== "unchanged").length
  const removed = removedGrantCount(draft, base.policy)
  const changeSummary = [
    changed > 0 ? `${changed} 条授权新增或修改` : null,
    removed > 0 ? `删除 ${removed} 条` : null,
    changed === 0 && removed === 0 && dirty ? "授权顺序已改" : null,
  ].filter((part) => part !== null)
  const globalIssues = issues.filter((issue) => issue.grantId === null)
  const grantIds = new Set(draft.grants.map((grant) => grant.id))
  const orphanDiagnostics = base.diagnostics.filter(
    (diagnostic) => diagnostic.rule_id === null || !grantIds.has(diagnostic.rule_id)
  )
  const editingId = builder?.session.initial.id ?? null
  const editingGrant = editingId ? draft.grants.find((grant) => grant.id === editingId) : undefined
  const full = draft.grants.length >= ACCESS_LIMITS.grants
  const usable = readyNodes(inventory).length

  return (
    <div data-testid="access-editor" className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-xl border bg-card px-4 py-2.5 text-xs">
        <Metric label="编辑基础">
          <span data-testid="access-base-revision" className="font-mono tabular-nums">
            r{base.revision}
          </span>
        </Metric>
        <Metric label="已保存">{base.policy.grants.length} 条授权</Metric>
        <Metric label="状态">
          {base.valid ? (
            <Badge variant="outline" className="text-success">有效</Badge>
          ) : (
            <Badge data-testid="access-base-invalid" variant="destructive">无效</Badge>
          )}
        </Metric>
        <Metric label="可用节点">
          <span className="tabular-nums" title="活动且已发布连接公钥">
            {usable}
          </span>
        </Metric>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="ml-auto"
          disabled={busy}
          onClick={requestReload}
        >
          <RefreshCwIcon data-icon="inline-start" className={reloading ? "animate-spin" : undefined} />
          {reloading ? "正在读取" : "重新读取"}
        </Button>
      </div>

      {!base.valid ? (
        <Alert variant="destructive">
          <TriangleAlertIcon />
          <AlertTitle>Controller 报告已保存的授权无效</AlertTitle>
          <AlertDescription className="flex flex-col gap-2">
            <p>
              无效时节点收到的有效授权为空，Mesh IPv4 流量全部拒绝。原授权保留在下方，修改或删除失效引用后重新保存。
            </p>
            <DiagnosticItems label="已保存授权的诊断" items={base.diagnostics} />
          </AlertDescription>
        </Alert>
      ) : null}

      {restoredNotice && restored ? (
        <Alert>
          <HistoryIcon />
          <AlertTitle>已恢复未保存的草稿</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>
              离开此页前的修改仍保留在本次会话中，编辑基础仍为 r{restored.base.revision}。刷新浏览器、退出登录会丢弃草稿。
            </p>
            <Button variant="ghost" size="sm" onClick={() => setRestoredNotice(false)}>
              知道了
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {behind ? (
        <Alert data-testid="access-behind">
          <TriangleAlertIcon />
          <AlertTitle>Controller 已有更新的配置版本 r{snapshot.revision}</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>草稿基于 r{base.revision}，编辑基础不会自动前移，此时保存会因版本冲突被拒绝。</p>
            <Button variant="outline" size="sm" disabled={busy} onClick={requestReload}>
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

      <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_22rem]">
        <Panel
          title={
            <span className="flex items-center gap-2">
              授权
              <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
                {draft.grants.length}/{ACCESS_LIMITS.grants}
              </span>
            </span>
          }
          description="每条授权是一条单向的完整路径。没有授权的路径一律拒绝。"
          actions={
            <Button
              type="button"
              size="sm"
              data-testid="access-add-grant"
              disabled={busy || full}
              onClick={newGrant}
            >
              <PlusIcon data-icon="inline-start" />
              添加授权
            </Button>
          }
        >
          {draft.grants.length === 0 ? (
            <Empty className="py-10">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <ShieldCheckIcon />
                </EmptyMedia>
                <EmptyTitle>还没有授权</EmptyTitle>
                <EmptyDescription>
                  默认拒绝：当前不放行任何 Mesh IPv4 流量。
                  {usable < 2 ? " 至少需要两个已发布连接公钥的活动节点。" : ""}
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button variant="outline" disabled={busy} onClick={newGrant}>
                  <PlusIcon data-icon="inline-start" />
                  添加第一条授权
                </Button>
              </EmptyContent>
            </Empty>
          ) : (
            <ol aria-label="访问授权" data-testid="access-grant-list" className="divide-y">
              {draft.grants.map((grant) => (
                <GrantRow
                  key={grant.id}
                  grant={grant}
                  inventory={inventory}
                  change={grantChange(grant, base.policy)}
                  problems={
                    issuesForGrant(issues, grant.id).length +
                    (grantChange(grant, base.policy) === "unchanged"
                      ? diagnosticsForGrant(base.diagnostics, grant.id).length
                      : 0)
                  }
                  reverse={hasReverse(grant, draft.grants)}
                  canAddReverse={!full && validatePath(grant.path, inventory).length === 0}
                  highlighted={highlight === grant.id}
                  disabled={busy}
                  onEdit={() => editGrant(grant)}
                  onAddReverse={() => addReverse(grant)}
                  onDelete={() => deleteGrant(grant.id)}
                />
              ))}
            </ol>
          )}

          {globalIssues.length > 0 || orphanDiagnostics.length > 0 ? (
            <div className="flex flex-col gap-2 border-t px-4 py-3">
              <DiagnosticItems label="策略级问题" items={globalIssues} />
              <DiagnosticItems label="已保存授权的其他诊断" items={orphanDiagnostics} />
            </div>
          ) : null}
        </Panel>

        <ScopePanel />
      </div>

      <SaveBar
        dirty={dirty}
        changeSummary={changeSummary}
        issueCount={issues.length}
        baseRevision={base.revision}
        saving={saving}
        disabled={busy}
        save={save}
        onSave={runSave}
        onRevert={() => setConfirm("revert")}
        onReload={requestReload}
      />

      <GrantBuilder
        session={builder?.session ?? null}
        open={builder?.open ?? false}
        dirty={working !== null}
        onWorkingChange={trackBuilderForm}
        onClose={closeBuilder}
        onApply={applyGrants}
        onDelete={() => {
          if (editingId) deleteGrant(editingId)
          closeBuilder()
        }}
        others={draft.grants.filter((grant) => grant.id !== editingId)}
        inventory={inventory}
        diagnostics={editingGrant ? diagnosticsForGrant(base.diagnostics, editingGrant.id) : []}
        diagnosticsStale={editingGrant ? grantChange(editingGrant, base.policy) !== "unchanged" : false}
        disabled={busy}
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
              {confirm === "revert" ? "放弃草稿中的修改？" : "放弃草稿并重新读取？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirm === "revert"
                ? `草稿恢复为已保存的 r${base.revision} 授权，编辑基础不变。`
                : "未保存的修改会被丢弃，替换为 controller 上的最新授权和节点公钥清单。"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <Button variant="destructive" onClick={confirm === "revert" ? revertDraft : runReload}>
              {confirm === "revert" ? "放弃修改" : "放弃草稿并重新读取"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function GrantRow({
  grant,
  inventory,
  change,
  problems,
  reverse,
  canAddReverse,
  highlighted,
  disabled,
  onEdit,
  onAddReverse,
  onDelete,
}: {
  grant: MeshAccessGrant
  inventory: ConnectionNode[]
  change: GrantChange
  problems: number
  reverse: boolean
  canAddReverse: boolean
  highlighted: boolean
  disabled: boolean
  onEdit: () => void
  onAddReverse: () => void
  onDelete: () => void
}) {
  const ref = useRef<HTMLLIElement>(null)
  const badge = changeBadge[change]

  useEffect(() => {
    if (highlighted) ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" })
  }, [highlighted])

  return (
    <li
      ref={ref}
      data-testid="access-grant-row"
      data-grant-id={grant.id}
      className={cn(
        "flex flex-col gap-2 px-3 py-3 transition-colors sm:flex-row sm:items-center sm:px-4",
        highlighted && "bg-brand/5"
      )}
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <GrantPath path={grant.path} inventory={inventory} />
        <div className="flex flex-wrap items-center gap-1.5 text-[0.6875rem]">
          {badge ? (
            <Badge variant="outline" className={badge.className}>
              {badge.label}
            </Badge>
          ) : null}
          {problems > 0 ? (
            <Badge data-testid="access-grant-problems" variant="destructive">
              <CircleAlertIcon data-icon="inline-start" />
              {problems} 个问题
            </Badge>
          ) : null}
          <span
            className={cn("inline-flex items-center gap-1", reverse ? "text-muted-foreground" : "text-warning")}
            title="回包与反向发起需要完全相反路径的授权，以及对应的反向路由规则。"
          >
            <ArrowLeftRightIcon className="size-3" />
            {reverse ? "已有反向授权" : "仅单向：无反向授权"}
          </span>
          <code className="font-mono text-muted-foreground" title={grant.id}>
            {grant.id.slice(0, 8)}
          </code>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1 self-end sm:self-center">
        <Button variant="ghost" size="sm" disabled={disabled} onClick={onEdit}>
          <PencilIcon data-icon="inline-start" />
          编辑
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button variant="ghost" size="icon-sm" aria-label="更多操作" disabled={disabled} />
            }
          >
            <EllipsisIcon />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem disabled={reverse || !canAddReverse} onClick={onAddReverse}>
              <ArrowLeftRightIcon />
              添加反向授权
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onClick={onDelete}>
              <Trash2Icon />
              删除
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </li>
  )
}

/** Read-only statement of what an access grant does in this slice. */
function ScopePanel() {
  return (
    <Panel
      className="xl:sticky xl:top-16"
      title="授权范围"
      bodyClassName="flex flex-col gap-3 p-4 text-xs"
    >
      <p data-testid="access-relationship" className="text-sm font-medium">
        路由决定走哪条路径，授权决定这条路径是否允许使用。
      </p>
      <ul className="flex flex-col gap-1.5 text-muted-foreground">
        <ScopeItem>只放行路径两端节点的 Mesh IPv4 地址之间的报文，全部 IPv4 协议与端口。</ScopeItem>
        <ScopeItem>路由编译出的路径必须与授权完全一致；顺序或经由节点不同都会被拒绝。</ScopeItem>
        <ScopeItem>单向。回包与反向发起需要反向授权，以及一条反向路由规则。</ScopeItem>
        <ScopeItem>不含 IPv6、DNS 与互联网出口。</ScopeItem>
        <ScopeItem>
          保存只表示配置已保存。由以 <code className="font-mono text-foreground">--apply-dataplane</code>{" "}
          运行的 Linux agent 执行；是否连通请用 ping / curl 实测。
        </ScopeItem>
      </ul>
      <div className="flex flex-wrap gap-x-4 gap-y-1 border-t pt-3">
        <Link
          href="/routing"
          className="inline-flex items-center gap-1 text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
        >
          路由策略
          <ArrowUpRightIcon className="size-3" />
        </Link>
        <a
          href={`${docsUrl}/managed-dataplane`}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
        >
          托管数据面文档
          <ArrowUpRightIcon className="size-3" />
        </a>
      </div>
    </Panel>
  )
}

function ScopeItem({ children }: { children: ReactNode }) {
  return (
    <li className="flex items-start gap-2">
      <span aria-hidden="true" className="mt-1.5 size-1 shrink-0 rounded-full bg-muted-foreground/60" />
      <span>{children}</span>
    </li>
  )
}

function DiagnosticItems({
  label,
  items,
}: {
  label: string
  items: { field: string; message: string }[]
}) {
  if (items.length === 0) return null
  return (
    <ul aria-label={label} className="flex flex-col gap-1.5 text-xs text-destructive">
      {items.map((item, index) => (
        <li key={index} className="flex items-start gap-1.5">
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 break-words">
            <span className="font-medium">{accessFieldLabel(item.field)}</span>：{item.message}
          </span>
        </li>
      ))}
    </ul>
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
  save: AccessSaveResult | null
  onSave: () => void
  onRevert: () => void
  onReload: () => void
}) {
  return (
    <div className="sticky bottom-3 z-10 flex flex-col gap-2 rounded-xl border bg-background/95 px-4 py-3 shadow-lg backdrop-blur supports-backdrop-filter:bg-background/80">
      <div data-testid="access-save-status" aria-live="polite">
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
            配置已保存为 r{save.snapshot.revision}
            {save.snapshot.valid
              ? "。节点同步后由启用托管数据面的 agent 使用；连通性请用 ping / curl 实测。"
              : "，但 controller 报告授权无效，见上方诊断。"}
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
                <span className="text-muted-foreground"> · {changeSummary.join("，")}</span>
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
            data-testid="access-save"
            disabled={disabled || !dirty || issueCount > 0}
            onClick={onSave}
            title={`条件保存：仅当 controller 仍为 r${baseRevision} 时写入`}
          >
            {saving ? (
              <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" />
            ) : (
              <SaveIcon data-icon="inline-start" />
            )}
            保存授权
          </Button>
        </div>
      </div>
    </div>
  )
}
