"use client"

import { useRoutes } from "@/components/shell/network-context"
import { useNetworkId } from "@/components/network/network-workspace"

import type {
  AccessAction,
  ConnectionNode,
  MeshAccessPolicy,
  MeshAccessRule,
  MeshAccessSnapshot,
  TagSnapshot,
} from "@meshway/control-client"
import { schemaFor } from "@meshway/control-client/json-schema"
import { zMeshAccessPolicy } from "@meshway/control-client/schemas"
import {
  BracesIcon,
  CircleAlertIcon,
  HistoryIcon,
  ListOrderedIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SaveIcon,
  ShieldCheckIcon,
  TagIcon,
  TriangleAlertIcon,
} from "lucide-react"
import Link from "next/link"
import { useEffect, useMemo, useRef, useState, useTransition, type ReactNode } from "react"

import { useAccessDraftStore, type Owner } from "@/components/access/draft-store"
import { FlowCheck } from "@/components/access/flow-check"
import { AccessRuleBuilder } from "@/components/access/rule-builder"
import { AccessRuleCard, type RuleCardState } from "@/components/access/rule-card"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { DraftActionBar } from "@/components/common/draft-action-bar"
import { Panel } from "@/components/common/panel"
import { Metric, StatusSummary } from "@/components/common/status-summary"
import { Segmented } from "@/components/common/segmented"
import { JsoncPane } from "@/components/policy/jsonc-pane"
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
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty"
import { Spinner } from "@/components/ui/spinner"
import { useToastManager } from "@/components/ui/toast"
import { reloadMeshAccessAction, saveMeshAccessAction } from "@/lib/access-actions"
import { accessBase, type AccessView } from "@/lib/access-draft-registry"
import {
  ACCESS_LIMITS,
  accessFieldLabel,
  actionLabels,
  activeInventory,
  diagnosticsForRule,
  issuesForRule,
  removedRuleCount,
  ruleChange,
  shadowedBy,
  suggestRuleId,
  validateAccess,
} from "@/lib/access-model"
import {
  newRuleForm,
  toRuleForm,
  updateRuleForm,
  type AccessRuleForm,
  type AccessRuleSession,
  type WorkingAccessRule,
} from "@/lib/access-rule-form"
import { ALL_SELECTOR_HINT, nodeSelector } from "@/lib/access-selector"
import type { AccessSaveResult } from "@/lib/action-state"
import type { ReferenceSuggestions } from "@/lib/jsonc-complete"
import {
  applyVisual,
  discardInvalidText,
  documentChange,
  editText,
  hasUnsavedWork,
  isValid,
  openDocument,
  restoreDocument,
  revertToBase,
  semanticProblems,
  settleSaved,
  storeDocument,
  type PolicyDocument,
} from "@/lib/policy-document"
import { cn } from "@/lib/utils"

const accessSchema = schemaFor("MeshAccessPolicy")

const viewOptions = [
  {
    value: "visual",
    label: (
      <span className="flex items-center gap-1.5">
        <ListOrderedIcon className="size-3.5" />
        规则
      </span>
    ),
  },
  {
    value: "jsonc",
    label: (
      <span className="flex items-center gap-1.5">
        <BracesIcon className="size-3.5" />
        JSONC
      </span>
    ),
  },
] as const satisfies readonly { value: AccessView; label: ReactNode }[]

function moveItem<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items]
  if (from === to || to < 0 || to >= items.length) return next
  const [moved] = next.splice(from, 1)
  if (moved === undefined) return [...items]
  next.splice(to, 0, moved)
  return next
}

/**
 * Endpoint access rules, edited as ordered first-match cards or as JSONC on
 * one shared draft. The CAS base only changes on a successful save or an
 * explicit reload; a newer controller revision is reported, never adopted.
 */
export function AccessEditor({
  snapshot,
  inventory: initialInventory,
  tags: initialTags,
  focusRuleId = null,
}: {
  snapshot: MeshAccessSnapshot
  inventory: ConnectionNode[]
  tags: TagSnapshot
  /** Rule to highlight on open (deep link from a flow preview). */
  focusRuleId?: string | null
}) {
  const drafts = useAccessDraftStore()
  const networkId = useNetworkId()
  const routes = useRoutes()
  const [restored] = useState(() => drafts.read())
  const [restoredNotice, setRestoredNotice] = useState(restored !== null)
  const [base, setBase] = useState(restored?.base ?? snapshot)
  const [inventory, setInventory] = useState(restored?.inventory ?? initialInventory)
  const [tags, setTags] = useState(restored?.tags ?? initialTags)
  const [document, setDocument] = useState<PolicyDocument<MeshAccessPolicy>>(() =>
    restored ? restoreDocument(restored.document, zMeshAccessPolicy) : openDocument(accessBase(snapshot), zMeshAccessPolicy)
  )
  const [view, setView] = useState<AccessView>(restored?.view ?? "visual")
  const [save, setSave] = useState<AccessSaveResult | null>(null)
  const [reloadError, setReloadError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<"reload" | "revert" | null>(null)
  const [working, setWorking] = useState<WorkingAccessRule | null>(restored?.builder ?? null)
  const [builder, setBuilder] = useState<{ session: AccessRuleSession; open: boolean } | null>(() =>
    restored?.builder ? { open: true, session: { ...restored.builder.session, working: restored.builder.form } } : null
  )
  const [highlight, setHighlight] = useState<string | null>(focusRuleId)
  const [saving, startSave] = useTransition()
  const [reloading, startReload] = useTransition()
  const generation = useRef(0)
  const undoToasts = useRef<string[]>([])
  const sessionId = useRef(restored?.nextSession ?? 0)
  const owner = useRef<Owner | null>(null)
  const reloadGeneration = useRef(0)
  const toast = useToastManager()

  useEffect(() => {
    const claimed = drafts.claim()
    owner.current = claimed
    const reloads = reloadGeneration
    return () => {
      reloads.current += 1
      owner.current = null
      drafts.release(claimed)
    }
  }, [drafts])

  useEffect(() => {
    if (highlight === null) return
    const timer = window.setTimeout(() => setHighlight(null), 2500)
    return () => window.clearTimeout(timer)
  }, [highlight])

  const policy = document.value
  const textValid = isValid(document)
  const change = documentChange(document)
  const dirty = change.valueChanged
  const unsaved = hasUnsavedWork(document)
  const issues = useMemo(() => validateAccess(policy, inventory, tags), [policy, inventory, tags])
  const textProblems = useMemo(
    () => [...document.problems, ...(textValid ? semanticProblems(document, issues) : [])],
    [document, issues, textValid]
  )
  const blocking = issues.length + (textValid ? 0 : document.problems.length)
  const busy = saving || reloading
  const visualLocked = busy || !textValid
  const behind = snapshot.revision > base.revision

  useEffect(() => {
    if (unsaved || working !== null) {
      drafts.write(owner.current, {
        base,
        inventory,
        tags,
        document: storeDocument(document),
        builder: working,
        nextSession: sessionId.current,
        view,
      })
    } else {
      drafts.clear(owner.current)
    }
  }, [unsaved, working, base, inventory, tags, document, view, drafts])

  /** Visual edits go through the JSONC document so comments elsewhere stay. */
  const updatePolicy = (update: (current: MeshAccessPolicy) => MeshAccessPolicy) => {
    setDocument((current) => {
      const result = applyVisual(current, update(current.value), zMeshAccessPolicy)
      return result.ok ? result.document : current
    })
    setSave(null)
  }
  const updateText = (text: string) => {
    setDocument((current) => editText(current, text, zMeshAccessPolicy))
    setSave(null)
  }

  const newGeneration = () => {
    generation.current += 1
    for (const id of undoToasts.current) toast.close(id)
    undoToasts.current = []
    setRestoredNotice(false)
  }

  const openBuilder = (mode: AccessRuleSession["mode"], position: number, initial: AccessRuleForm) => {
    sessionId.current += 1
    setWorking(null)
    setBuilder({ session: { id: sessionId.current, mode, position, initial }, open: true })
  }
  const closeBuilder = () => {
    setWorking(null)
    setBuilder((current) => (current ? { ...current, open: false } : null))
  }
  const trackBuilderForm = (form: AccessRuleForm | null) => {
    const session = builder?.session
    if (!session) return
    const opened = { id: session.id, mode: session.mode, position: session.position, initial: session.initial }
    setWorking(form === null ? null : { session: opened, form })
  }
  const takenIds = (except: number | null) =>
    policy.rules.filter((_, index) => index !== except).map((rule) => rule.id)

  const newRule = () =>
    openBuilder("new", policy.rules.length, updateRuleForm(newRuleForm(), {}, takenIds(null), inventory))
  const editRule = (index: number) => {
    const rule = policy.rules[index]
    if (rule) openBuilder("edit", index, toRuleForm(rule))
  }
  const applyRule = (rule: MeshAccessRule) => {
    const session = builder?.session
    if (!session) return
    updatePolicy((current) => {
      const rules = [...current.rules]
      if (session.mode === "edit") rules.splice(session.position, 1, rule)
      else rules.splice(Math.min(session.position, rules.length), 0, rule)
      return { ...current, rules }
    })
    setHighlight(rule.id)
    closeBuilder()
  }
  const duplicateRule = (index: number) => {
    const rule = policy.rules[index]
    if (!rule || policy.rules.length >= ACCESS_LIMITS.rules) return
    const next = { ...rule, id: suggestRuleId(rule, takenIds(null), inventory), source: [...rule.source], destination: [...rule.destination] }
    updatePolicy((current) => {
      const rules = [...current.rules]
      rules.splice(index + 1, 0, next)
      return { ...current, rules }
    })
    setHighlight(next.id)
  }
  const deleteRule = (index: number) => {
    const rule = policy.rules[index]
    if (!rule) return
    updatePolicy((current) => ({ ...current, rules: current.rules.filter((_, position) => position !== index) }))
    const owned = generation.current
    const toastId = toast.add({
      title: `已从草稿删除规则 ${rule.id}`,
      description: "保存前不会影响 controller。",
      actionProps: {
        children: "撤销",
        onClick: () => {
          undoToasts.current = undoToasts.current.filter((existing) => existing !== toastId)
          if (owned !== generation.current) return
          updatePolicy((current) => {
            if (current.rules.some((candidate) => candidate.id === rule.id)) return current
            const rules = [...current.rules]
            rules.splice(Math.min(index, rules.length), 0, rule)
            return { ...current, rules }
          })
        },
      },
    })
    undoToasts.current = [...undoToasts.current, toastId]
  }
  const setDefault = (defaultAction: AccessAction) => updatePolicy((current) => ({ ...current, default_action: defaultAction }))

  const runSave = () => {
    const expected = base.revision
    const body = policy
    const source = document.text
    const sentSeq = document.seq
    const saver = owner.current
    startSave(async () => {
      const result = await saveMeshAccessAction(networkId, expected, body, source)
      if (!drafts.owns(saver)) {
        if (result.status === "saved") drafts.adoptOrphanSave(expected, sentSeq, result.snapshot, zMeshAccessPolicy)
        return
      }
      setSave(result)
      if (result.status === "saved") {
        newGeneration()
        setBase(result.snapshot)
        setDocument((current) => settleSaved(current, accessBase(result.snapshot), sentSeq, zMeshAccessPolicy))
      }
    })
  }

  const runReload = () => {
    setConfirm(null)
    const requested = ++reloadGeneration.current
    const loader = owner.current
    startReload(async () => {
      const result = await reloadMeshAccessAction(networkId)
      if (requested !== reloadGeneration.current || !drafts.owns(loader)) return
      if (result.status === "error") {
        setReloadError(result.message)
        return
      }
      setReloadError(null)
      setWorking(null)
      setBuilder((current) => (current ? { ...current, open: false } : null))
      newGeneration()
      setBase(result.snapshot)
      setInventory(result.inventory)
      setTags(result.tags)
      setDocument(openDocument(accessBase(result.snapshot), zMeshAccessPolicy))
      setSave(null)
      setHighlight(null)
    })
  }
  const requestReload = () => (unsaved ? setConfirm("reload") : runReload())
  const revertDraft = () => {
    setConfirm(null)
    newGeneration()
    setDocument((current) => revertToBase(current, zMeshAccessPolicy))
    setSave(null)
  }

  const states: RuleCardState[] = policy.rules.map((rule, index) => {
    const shadow = shadowedBy(policy, index)
    return {
      change: ruleChange(rule, base.policy),
      problems:
        issuesForRule(issues, index).length +
        (ruleChange(rule, base.policy) === "unchanged" ? diagnosticsForRule(base.diagnostics, rule.id).length : 0),
      shadowedBy: shadow === null ? null : shadow + 1,
    }
  })
  const changed = states.filter((state) => state.change !== "unchanged").length
  const removed = removedRuleCount(policy, base.policy)
  const defaultChanged = policy.default_action !== base.policy.default_action
  const changeSummary = [
    defaultChanged ? `默认动作改为${actionLabels[policy.default_action]}` : null,
    changed > 0 ? `${changed} 条规则新增或修改` : null,
    removed > 0 ? `删除 ${removed} 条` : null,
    changed === 0 && removed === 0 && !defaultChanged && dirty ? "规则顺序已改" : null,
  ].filter((part) => part !== null)
  const globalIssues = issues.filter((issue) => issue.ruleIndex === null)
  const ruleIds = new Set(policy.rules.map((rule) => rule.id))
  const orphanDiagnostics = base.diagnostics.filter(
    (diagnostic) => diagnostic.rule_id === null || !ruleIds.has(diagnostic.rule_id)
  )
  const editing = builder?.session.mode === "edit" ? builder.session.position : null
  const editingRule = editing === null ? undefined : policy.rules[editing]
  const full = policy.rules.length >= ACCESS_LIMITS.rules
  const active = activeInventory(inventory)
  const nodes = inventory.map((entry) => entry.node)

  const references: ReferenceSuggestions = {
    selector: [
      { value: "*", label: "*", detail: "全部已注册节点" },
      ...tags.tags.map((definition) => ({ value: definition.tag, label: definition.tag, detail: definition.description || "标签" })),
      ...active.map((entry) => ({
        value: nodeSelector(entry.node.id),
        label: entry.node.name,
        detail: `${entry.node.mesh_ipv4} · ${entry.node.id.slice(0, 8)}`,
      })),
    ],
    node: active.map((entry) => ({ value: entry.node.id, label: entry.node.name, detail: entry.node.mesh_ipv4 })),
    tag: tags.tags.map((definition) => ({ value: definition.tag, label: definition.tag, detail: definition.description })),
  }

  const viewSwitch = (
    <Segmented size="sm" aria-label="编辑方式" value={view} onValueChange={setView} options={viewOptions} />
  )

  return (
    <div data-testid="access-editor" className="flex flex-col gap-4">
      <StatusSummary>
        <Metric label="编辑基础">
          <span data-testid="access-base-revision" className="font-mono tabular-nums">
            r{base.revision}
          </span>
        </Metric>
        <Metric label="已保存">
          {base.policy.rules.length} 条规则 · 默认{actionLabels[base.policy.default_action]}
        </Metric>
        <Metric label="状态">
          {base.valid ? (
            <Badge variant="outline" className="text-success">
              有效
            </Badge>
          ) : (
            <Badge data-testid="access-base-invalid" variant="destructive">
              无效
            </Badge>
          )}
        </Metric>
        <Metric label="已注册节点">
          <span className="tabular-nums">{active.length}</span>
        </Metric>
        <Link href={routes.tags()} className="flex items-center gap-1 text-muted-foreground hover:text-foreground">
          <TagIcon className="size-3.5" />
          {tags.tags.length} 个标签
        </Link>
        <Button type="button" variant="ghost" size="sm" className="ml-auto" disabled={busy} onClick={requestReload}>
          <RefreshCwIcon data-icon="inline-start" className={reloading ? "animate-spin" : undefined} />
          {reloading ? "正在读取" : "重新读取"}
        </Button>
      </StatusSummary>

      {!base.valid ? (
        <Alert variant="destructive">
          <TriangleAlertIcon />
          <AlertTitle>Controller 报告已保存的授权无效</AlertTitle>
          <AlertDescription className="flex flex-col gap-2">
            <p>无效时节点收到的有效授权为空，Mesh IPv4 流量全部拒绝。原规则保留在下方，修改或删除失效引用后重新保存。</p>
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
            <p>草稿基于 r{base.revision}（标签、节点或路由的修改也会推进版本），此时保存会因版本冲突被拒绝。</p>
            <Button variant="outline" size="sm" disabled={busy} onClick={requestReload}>
              载入最新版本{unsaved ? "（先确认放弃草稿）" : ""}
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

      {!textValid && view === "visual" ? (
        <Alert variant="destructive" data-testid="access-text-invalid">
          <TriangleAlertIcon />
          <AlertTitle>JSONC 文本有 {document.problems.length} 个问题，规则编辑已暂停</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>下方是最后一次有效的规则，只读显示。修复文本，或明确放弃文本中尚未生效的修改；切换视图不会丢弃它们。</p>
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={() => setView("jsonc")}>
                回到 JSONC 修复
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setDocument((current) => discardInvalidText(current, zMeshAccessPolicy))}
              >
                放弃无效的文本修改
              </Button>
            </div>
          </AlertDescription>
        </Alert>
      ) : null}

      <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_24rem]">
        {view === "jsonc" ? (
          <Panel
            title={
              <span className="flex items-center gap-2">
                访问授权 JSONC
                {!textValid ? <Badge variant="destructive">{document.problems.length} 个问题</Badge> : null}
              </span>
            }
            description="与规则视图共用同一份草稿；注释随策略保存。"
            actions={viewSwitch}
          >
            <JsoncPane
              label="访问授权 JSONC"
              text={document.text}
              problems={textProblems}
              onChange={updateText}
              schemaRoot={accessSchema}
              schema={accessSchema}
              references={references}
              readOnly={busy}
              notice={change.textOnly ? <>只有注释或排版改动：保存后 controller 会连同原文一起存储，规则不变。</> : null}
            />
          </Panel>
        ) : (
          <Panel
            title={
              <span className="flex items-center gap-2">
                访问规则
                <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
                  {policy.rules.length}/{ACCESS_LIMITS.rules}
                </span>
              </span>
            }
            actions={
              <>
                {viewSwitch}
                <Button
                  type="button"
                  size="sm"
                  data-testid="access-add-rule"
                  disabled={visualLocked || full}
                  onClick={newRule}
                >
                  <PlusIcon data-icon="inline-start" />
                  添加规则
                </Button>
              </>
            }
          >
            <DefaultActionCard
              value={policy.default_action}
              changed={defaultChanged}
              disabled={visualLocked}
              onChange={setDefault}
            />
            {policy.rules.length === 0 ? (
              <Empty className="py-10">
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <ShieldCheckIcon />
                  </EmptyMedia>
                  <EmptyTitle>还没有规则</EmptyTitle>
                  <EmptyDescription>
                    所有 Mesh IPv4 流量都按默认动作{actionLabels[policy.default_action]}。添加规则为特定来源和目标设置例外。
                  </EmptyDescription>
                </EmptyHeader>
                <EmptyContent>
                  <Button variant="outline" disabled={visualLocked} onClick={newRule}>
                    <PlusIcon data-icon="inline-start" />
                    添加第一条规则
                  </Button>
                </EmptyContent>
              </Empty>
            ) : (
              <ol aria-label="访问规则（按匹配顺序）" data-testid="access-rule-list" className="divide-y border-t">
                {policy.rules.map((rule, index) => (
                  <AccessRuleCard
                    key={`${index}-${rule.id}`}
                    rule={rule}
                    index={index}
                    count={policy.rules.length}
                    state={states[index] ?? { change: "unchanged", problems: 0, shadowedBy: null }}
                    inventory={inventory}
                    tags={tags}
                    highlighted={highlight === rule.id}
                    disabled={visualLocked}
                    onEdit={() => editRule(index)}
                    onMove={(target) => updatePolicy((current) => ({ ...current, rules: moveItem(current.rules, index, target) }))}
                    onDuplicate={() => duplicateRule(index)}
                    onDelete={() => deleteRule(index)}
                  />
                ))}
              </ol>
            )}
            <div className="flex items-center gap-3 border-t bg-muted/30 px-3 py-2.5 text-xs text-muted-foreground sm:px-4">
              <span className="flex size-6 shrink-0 items-center justify-center rounded-md border border-dashed font-mono text-[0.6875rem]">
                ∗
              </span>
              其余流量：
              <Badge
                variant="outline"
                className={policy.default_action === "allow" ? "border-success/50 text-success" : "border-destructive/50 text-destructive"}
              >
                默认{actionLabels[policy.default_action]}
              </Badge>
            </div>

            {globalIssues.length > 0 || orphanDiagnostics.length > 0 ? (
              <div className="flex flex-col gap-2 border-t px-4 py-3">
                <DiagnosticItems label="策略级问题" items={globalIssues} />
                <DiagnosticItems label="已保存授权的其他诊断" items={orphanDiagnostics} />
              </div>
            ) : null}
          </Panel>
        )}

        <div className="flex min-w-0 flex-col gap-4 xl:sticky xl:top-16">
          <FlowCheck nodes={nodes} baseRevision={base.revision} dirty={dirty} />
          <ScopePanel />
        </div>
      </div>

      <SaveBar
        dirty={dirty}
        textOnly={change.textOnly}
        changeSummary={changeSummary}
        issueCount={blocking}
        baseRevision={base.revision}
        saving={saving}
        disabled={busy}
        save={save}
        onSave={runSave}
        onRevert={() => setConfirm("revert")}
        onReload={requestReload}
      />

      <AccessRuleBuilder
        session={builder?.session ?? null}
        open={builder?.open ?? false}
        dirty={working !== null}
        onWorkingChange={trackBuilderForm}
        onClose={closeBuilder}
        onApply={applyRule}
        onDelete={() => {
          if (editing !== null) deleteRule(editing)
          closeBuilder()
        }}
        others={policy.rules.filter((_, index) => index !== editing)}
        inventory={inventory}
        tags={tags}
        diagnostics={editingRule ? diagnosticsForRule(base.diagnostics, editingRule.id) : []}
        diagnosticsStale={editingRule ? ruleChange(editingRule, base.policy) !== "unchanged" : false}
        disabled={visualLocked}
      />

      <AlertDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open) setConfirm(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{confirm === "revert" ? "放弃草稿中的修改？" : "放弃草稿并重新读取？"}</AlertDialogTitle>
            <AlertDialogDescription>
              {confirm === "revert"
                ? `草稿（含 JSONC 文本与注释）恢复为已保存的 r${base.revision}，编辑基础不变。`
                : "未保存的修改会被丢弃，替换为 controller 上的最新授权、节点和标签。"}
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

function DefaultActionCard({
  value,
  changed,
  disabled,
  onChange,
}: {
  value: AccessAction
  changed: boolean
  disabled: boolean
  onChange: (value: AccessAction) => void
}) {
  return (
    <div
      data-testid="access-default-action"
      className={cn(
        "flex flex-col gap-3 px-3 py-3 sm:flex-row sm:items-center sm:px-4",
        value === "allow" ? "bg-success/5" : "bg-destructive/5"
      )}
    >
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex items-center gap-2 text-sm font-medium">
          默认动作
          {changed ? <Badge variant="secondary">已修改</Badge> : null}
        </span>
        <span className="text-xs text-muted-foreground">
          {value === "allow"
            ? "没有规则命中的 Mesh IPv4 流量一律允许，包括之后注册的节点。用拒绝规则设置例外。"
            : "没有规则命中的 Mesh IPv4 流量一律拒绝。用允许规则放行需要的访问。"}
        </span>
      </div>
      <Segmented
        aria-label="默认动作"
        value={value}
        onValueChange={onChange}
        disabled={disabled}
        options={[
          { value: "reject", label: "拒绝" },
          { value: "allow", label: "允许" },
        ]}
      />
    </div>
  )
}

/** What an access rule does and does not do; folded, the editor is the task. */
function ScopePanel() {
  const routes = useRoutes()
  return (
    <DiagnosticDetails title="规则如何生效" className="bg-card shadow-xs">
      <p data-testid="access-relationship" className="font-medium text-foreground">
        访问授权只决定端点之间能否访问；走哪条路径由
        <Link href={routes.routing()} className="mx-0.5 underline underline-offset-4">路由</Link>
        决定。
      </p>
      <ul className="flex flex-col gap-1.5 text-muted-foreground">
        <ScopeItem>{ALL_SELECTOR_HINT}</ScopeItem>
        <ScopeItem>
          <Link href={routes.tags()} className="underline underline-offset-4">标签</Link>
          由管理员维护；成员变化随配置版本生效。
        </ScopeItem>
        <ScopeItem>双向规则在同一位置也匹配反向流量，两个方向各自检查；没有状态型回包跟踪。</ScopeItem>
        <ScopeItem>允许不会绕过路由的拒绝或必经节点；controller 据此为每条路径编译内部授权。</ScopeItem>
        <ScopeItem>未发布公钥的节点规则仍然有效，只是数据面在公钥发布前不执行。</ScopeItem>
        <ScopeItem>引用已撤销 / 未知的节点或不存在的标签会使整个授权无效（全部拒绝）。</ScopeItem>
      </ul>
    </DiagnosticDetails>
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

function DiagnosticItems({ label, items }: { label: string; items: { field: string; message: string }[] }) {
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

function SaveBar({
  dirty,
  textOnly,
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
  textOnly: boolean
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
  const pending = dirty || textOnly
  return (
    <DraftActionBar visible={pending || saving || issueCount > 0 || save !== null}>
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
      <div className="draft-action-bar-row">
        <div className="draft-action-bar-summary text-xs">
          <span aria-hidden="true" className={cn("size-2 shrink-0 rounded-full", pending ? "bg-warning" : "bg-muted-foreground/30")} />
          <span className="min-w-0">
            {dirty ? (
              <>
                <span className="font-medium">未保存的草稿</span>
                <span className="text-muted-foreground"> · {changeSummary.join("，")}</span>
              </>
            ) : textOnly ? (
              <>
                <span className="font-medium">未保存的 JSONC 原文</span>
                <span className="text-muted-foreground"> · 只有注释或排版不同</span>
              </>
            ) : (
              <span className="text-muted-foreground">草稿与已保存的 r{baseRevision} 一致</span>
            )}
            {issueCount > 0 ? <span className="text-destructive"> · {issueCount} 个问题需修复</span> : null}
          </span>
        </div>
        <div className="draft-action-bar-actions">
          <Button
            className={pending || issueCount > 0 ? undefined : "invisible"}
            variant="ghost"
            disabled={disabled || !(pending || issueCount > 0)}
            onClick={onRevert}
          >
            <RotateCcwIcon data-icon="inline-start" />
            放弃修改
          </Button>
          <Button
            size="lg"
            data-testid="access-save"
            disabled={disabled || !pending || issueCount > 0}
            onClick={onSave}
            title={`条件保存：仅当 controller 仍为 r${baseRevision} 时写入`}
          >
            {saving ? <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" /> : <SaveIcon data-icon="inline-start" />}
            保存授权
          </Button>
        </div>
      </div>
    </DraftActionBar>
  )
}
