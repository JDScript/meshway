"use client"

import type {
  NodeSummary,
  PolicyDiagnostic,
  PolicySnapshot,
} from "@meshway/control-client"
import { schemaFor } from "@meshway/control-client/json-schema"
import { zRoutingPolicy } from "@meshway/control-client/schemas"
import {
  BracesIcon,
  CircleAlertIcon,
  HistoryIcon,
  ListOrderedIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SaveIcon,
  TriangleAlertIcon,
} from "lucide-react"
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useTransition,
  type ReactNode,
} from "react"

import { OptionSelect } from "@/components/common/option-select"
import { DraftActionBar } from "@/components/common/draft-action-bar"
import { Panel } from "@/components/common/panel"
import { Segmented } from "@/components/common/segmented"
import { JsoncPane } from "@/components/policy/jsonc-pane"
import { DiagnosticList } from "@/components/routing/diagnostics"
import { RuleBuilder } from "@/components/routing/rule-builder"
import { RuleList, type RuleRowState } from "@/components/routing/rule-list"
import {
  FlowForm,
  PreviewButton,
  PreviewOutcome,
} from "@/components/routing/route-preview"
import { useRoutingDraftStore, type Owner } from "@/components/routing/draft-store"
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
import { Label } from "@/components/ui/label"
import { Spinner } from "@/components/ui/spinner"
import { useToastManager } from "@/components/ui/toast"
import type { PreviewResult, SaveResult } from "@/lib/action-state"
import { documentChange, hasUnsavedWork, isValid } from "@/lib/policy-document"
import {
  previewPolicyAction,
  reloadPolicyAction,
  savePolicyAction,
} from "@/lib/policy-actions"
import {
  diagnosticsForRule,
  emptyFlow,
  isCatchAll,
  POLICY_LIMITS,
  policyKey,
  removedRuleCount,
  ruleChange,
  toFlow,
  validateDraft,
  validateFlow,
  type DefaultAction,
  type FlowDraft,
  type PolicyDraft,
  type RuleDraft,
} from "@/lib/policy-model"
import {
  newRuleForm,
  toRuleForm,
  type BuilderSession,
  type RuleForm,
  type WorkingRule,
} from "@/lib/rule-form"
import type { RoutingView } from "@/lib/routing-draft-registry"
import {
  applyDraft,
  discardWorkspaceText,
  editWorkspaceText,
  localIssueProblems,
  openWorkspace,
  restoreWorkspace,
  revertWorkspace,
  routingBase,
  settleWorkspaceSave,
  storeWorkspace,
  workspaceDraft,
  type RoutingWorkspace,
} from "@/lib/routing-workspace"
import { cn } from "@/lib/utils"

const routingSchema = schemaFor("RoutingPolicy")

const viewOptions = [
  {
    value: "visual",
    label: (
      <span className="flex items-center gap-1.5">
        <ListOrderedIcon className="size-3.5" />
        图形
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
] as const satisfies readonly { value: RoutingView; label: ReactNode }[]

/** Diagnostics together with the exact policy they were compiled for. */
type Compiled = {
  source: "saved" | "preview"
  key: string
  valid: boolean
  diagnostics: PolicyDiagnostic[]
}

const fromSnapshot = (snapshot: PolicySnapshot): Compiled => ({
  source: "saved",
  key: policyKey(snapshot.policy),
  valid: snapshot.valid,
  diagnostics: snapshot.diagnostics,
})

const defaultActionOptions = [
  { value: "reject", label: "拒绝" },
  { value: "direct", label: "直连" },
] as const satisfies readonly { value: DefaultAction; label: string }[]

function uniqueRuleId(rules: RuleDraft[], stem = "rule"): string {
  const taken = new Set(rules.map((rule) => rule.id.trim()))
  let n = stem === "rule" ? rules.length + 1 : 1
  const make = (i: number) => (stem === "rule" ? `rule-${i}` : `${stem}-${i}`)
  while (taken.has(make(n))) n += 1
  return make(n)
}

function moveItem<T>(items: T[], from: number, to: number): T[] {
  if (from === to || to < 0 || to >= items.length) return items
  const next = [...items]
  const [moved] = next.splice(from, 1)
  if (moved === undefined) return items
  next.splice(to, 0, moved)
  return next
}

/**
 * The editing base (`base`) only ever changes on a successful save or an
 * explicit reload. Preview responses and page refreshes may reveal a newer
 * controller revision, which is reported but never adopted silently: the
 * conditional save always sends the revision the draft was loaded from.
 */
export function PolicyEditor({
  snapshot,
  nodes: initialNodes,
  focusRuleId = null,
}: {
  snapshot: PolicySnapshot
  nodes: NodeSummary[]
  /** Rule to highlight on open (deep link from a plan); not an edit. */
  focusRuleId?: string | null
}) {
  const drafts = useRoutingDraftStore()
  // A draft kept from an earlier visit brings back its own base snapshot; the
  // page's fresher snapshot then only shows up as "controller is ahead".
  const [restored] = useState(() => drafts.read())
  const [restoredNotice, setRestoredNotice] = useState(restored !== null)
  const [base, setBase] = useState(restored?.base ?? snapshot)
  const [nodes, setNodes] = useState(restored?.nodes ?? initialNodes)
  // The one draft: JSONC text (with comments) plus the last valid policy.
  // The visual editor is derived from it and writes back through it.
  const [workspace, setWorkspace] = useState<RoutingWorkspace>(() =>
    restored
      ? restoreWorkspace(restored.workspace, zRoutingPolicy)
      : openWorkspace(routingBase(snapshot), zRoutingPolicy)
  )
  const [view, setView] = useState<RoutingView>(restored?.view ?? "visual")
  const draft = useMemo(() => workspaceDraft(workspace), [workspace])
  const [compiled, setCompiled] = useState<Compiled>(() => fromSnapshot(restored?.base ?? snapshot))
  const [flow, setFlow] = useState<FlowDraft>(emptyFlow)
  // The flow is captured with its result: history renders what was asked.
  const [preview, setPreview] = useState<{
    result: PreviewResult
    key: string
    flow: FlowDraft
  } | null>(null)
  const [save, setSave] = useState<SaveResult | null>(null)
  const [reloadError, setReloadError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<"reload" | "revert" | null>(null)
  // Unapplied builder edits come back too: the builder reopens with them.
  const [working, setWorking] = useState<WorkingRule | null>(restored?.builder ?? null)
  const [builder, setBuilder] = useState<{ session: BuilderSession; open: boolean } | null>(() =>
    restored?.builder
      ? { open: true, session: { ...restored.builder.session, working: restored.builder.form } }
      : null
  )
  const [highlight, setHighlight] = useState<string | null>(() =>
    focusRuleId === null ? null : (draft.rules.find((rule) => rule.id.trim() === focusRuleId)?.key ?? null)
  )
  const [previewing, startPreview] = useTransition()
  const [saving, startSave] = useTransition()
  const [reloading, startReload] = useTransition()
  const nextKey = useRef(restored?.nextKey ?? 0)
  // Bumped whenever the draft is replaced wholesale (save, reload, revert).
  // Undo callbacks from an older generation must not touch the new draft.
  const generation = useRef(0)
  const undoToasts = useRef<string[]>([])
  const sessionId = useRef(restored?.nextSession ?? 0)
  const previewSeq = useRef(0)
  const defaultActionId = useId()
  const toast = useToastManager()
  // This mount's claim on the shared store, and the current reload request:
  // completions from an unmounted editor never touch a newer editor's work.
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
    if (highlight === null) return
    const timer = window.setTimeout(() => setHighlight(null), 2500)
    return () => window.clearTimeout(timer)
  }, [highlight])

  const policyDoc = workspace.document
  const policy = policyDoc.value
  const currentKey = policyKey(policy)
  const issues = useMemo(() => validateDraft(draft), [draft])
  const textValid = isValid(policyDoc)
  const change = documentChange(policyDoc)
  const dirty = change.valueChanged
  const unsaved = hasUnsavedWork(policyDoc)
  const textProblems = useMemo(
    () => [...workspace.document.problems, ...localIssueProblems(workspace, issues)],
    [workspace, issues]
  )
  const blocking = issues.length + (textValid ? 0 : policyDoc.problems.length)
  const busy = previewing || saving || reloading
  const locked = saving || reloading
  // While the JSONC buffer is invalid the visual editor shows the last valid
  // policy read-only: editing it would have to overwrite the typed text.
  const visualLocked = locked || !textValid
  const previewKey = JSON.stringify([currentKey, toFlow(flow)])
  const flowError = validateFlow(flow)

  const previewRevision =
    preview?.result.status === "previewed" ? preview.result.response.revision : 0
  const observedRevision = Math.max(snapshot.revision, previewRevision)
  const behind = observedRevision > base.revision

  // In-app navigation keeps the draft in the console-scoped store, which
  // also owns the reload / tab-close warning.
  useEffect(() => {
    if (unsaved || working !== null) {
      drafts.write(owner.current, {
        base,
        nodes,
        workspace: storeWorkspace(workspace),
        builder: working,
        nextKey: nextKey.current,
        nextSession: sessionId.current,
        view,
      })
    } else {
      drafts.clear(owner.current)
    }
  }, [unsaved, working, base, nodes, workspace, view, drafts])

  const compiledFresh = compiled.key === currentKey
  const states: RuleRowState[] = useMemo(() => {
    const firstCatchAll = draft.rules.findIndex(isCatchAll)
    const catchAllRule = draft.rules[firstCatchAll]
    return draft.rules.map((rule, index) => ({
      change: ruleChange(rule, base.policy),
      issues: issues.filter((issue) => issue.ruleKey === rule.key).length,
      diagnostics: diagnosticsForRule(compiled.diagnostics, rule.id).length,
      diagnosticsStale: !compiledFresh,
      shadowedBy:
        catchAllRule && index > firstCatchAll ? `第 ${firstCatchAll + 1} 条` : null,
    }))
  }, [draft.rules, base.policy, issues, compiled.diagnostics, compiledFresh])

  /** Visual edits go through the JSONC document so comments elsewhere stay. */
  const updateDraft = (update: (current: PolicyDraft) => PolicyDraft) => {
    setWorkspace((current) => {
      const result = applyDraft(current, update(workspaceDraft(current)), zRoutingPolicy)
      return result.ok ? result.workspace : current
    })
    setSave(null)
  }
  const updateText = (text: string) => {
    setWorkspace((current) => editWorkspaceText(current, text, zRoutingPolicy))
    setSave(null)
  }
  const discardText = () => setWorkspace((current) => discardWorkspaceText(current, zRoutingPolicy))

  const openBuilder = (session: Omit<BuilderSession, "id">) => {
    sessionId.current += 1
    setWorking(null)
    setBuilder({ session: { ...session, id: sessionId.current }, open: true })
  }
  /** Ends the session; its unapplied edits go, the outer draft stays. */
  const closeBuilder = () => {
    setWorking(null)
    setBuilder((current) => (current ? { ...current, open: false } : null))
  }
  const trackBuilderForm = (form: RuleForm | null) => {
    const session = builder?.session
    if (!session) return
    const opened = { id: session.id, mode: session.mode, initial: session.initial, position: session.position }
    setWorking(form === null ? null : { session: opened, form })
  }

  const editRule = (index: number) => {
    const rule = draft.rules[index]
    if (rule) openBuilder({ mode: "edit", initial: toRuleForm(rule), position: index })
  }
  const newRule = () =>
    openBuilder({
      mode: "new",
      initial: newRuleForm(`new-${nextKey.current++}`, uniqueRuleId(draft.rules)),
      position: draft.rules.length,
    })

  const applyRule = (rule: RuleDraft) => {
    const session = builder?.session
    if (!session) return
    updateDraft((current) => {
      const existing = current.rules.findIndex((candidate) => candidate.key === rule.key)
      if (existing >= 0) {
        return { ...current, rules: current.rules.map((r, i) => (i === existing ? rule : r)) }
      }
      const rules = [...current.rules]
      rules.splice(Math.min(session.position, rules.length), 0, rule)
      return { ...current, rules }
    })
    setHighlight(rule.key)
    closeBuilder()
  }

  /**
   * Wholesale draft or base replacement: rule keys may now mean different
   * rules, so pending undo actions are invalidated and their toasts closed.
   */
  const newGeneration = () => {
    generation.current += 1
    for (const id of undoToasts.current) toast.close(id)
    undoToasts.current = []
    setRestoredNotice(false)
  }

  const deleteRule = (key: string) => {
    const index = draft.rules.findIndex((rule) => rule.key === key)
    const rule = draft.rules[index]
    if (!rule) return
    updateDraft((current) => ({
      ...current,
      rules: current.rules.filter((candidate) => candidate.key !== key),
    }))
    const owner = generation.current
    const id = toast.add({
      title: `已从草稿删除规则 ${rule.id.trim() || `#${index + 1}`}`,
      description: "保存前不会影响 controller。",
      actionProps: {
        children: "撤销",
        onClick: () => {
          undoToasts.current = undoToasts.current.filter((existing) => existing !== id)
          // A save, reload or revert replaced the draft since the deletion.
          if (owner !== generation.current) return
          updateDraft((current) => {
            if (owner !== generation.current) return current
            if (current.rules.some((candidate) => candidate.key === key)) return current
            const rules = [...current.rules]
            rules.splice(Math.min(index, rules.length), 0, rule)
            return { ...current, rules }
          })
        },
      },
    })
    undoToasts.current = [...undoToasts.current, id]
  }

  const duplicateRule = (index: number) => {
    const rule = draft.rules[index]
    if (!rule || draft.rules.length >= POLICY_LIMITS.rules) return
    const key = `new-${nextKey.current++}`
    const copy: RuleDraft = {
      ...rule,
      key,
      id: uniqueRuleId(draft.rules, `${rule.id.trim() || "rule"}-copy`),
      destinations: rule.destinations.map((selector, i) => ({ ...selector, key: `${key}-d${i}` })),
      sourceNodeIds: [...rule.sourceNodeIds],
      via: [...rule.via],
    }
    updateDraft((current) => {
      const rules = [...current.rules]
      rules.splice(index + 1, 0, copy)
      return { ...current, rules }
    })
    setHighlight(key)
  }

  const runPreview = () => {
    const seq = ++previewSeq.current
    const key = previewKey
    const body = policy
    const compiledKey = currentKey
    const query = toFlow(flow)
    const submittedFlow = flow
    startPreview(async () => {
      const result = await previewPolicyAction(body, query)
      // A newer preview or a reload supersedes this response.
      if (seq !== previewSeq.current) return
      setPreview({ result, key, flow: submittedFlow })
      if (result.status === "previewed") {
        setCompiled({
          source: "preview",
          key: compiledKey,
          valid: result.response.valid,
          diagnostics: result.response.diagnostics,
        })
      }
    })
  }

  const runSave = () => {
    const expected = base.revision
    const body = policy
    const source = policyDoc.text
    const sentSeq = policyDoc.seq
    const saver = owner.current
    startSave(async () => {
      const result = await savePolicyAction(expected, body, source)
      if (!drafts.owns(saver)) {
        // This editor unmounted meanwhile: kept work that is exactly what
        // was saved moves onto the new base; anything else is left alone.
        if (result.status === "saved") {
          const saved = { snapshot: result.snapshot, base: routingBase(result.snapshot) }
          drafts.adoptOrphanSave(expected, sentSeq, saved, zRoutingPolicy)
        }
        return
      }
      setSave(result)
      if (result.status === "saved") {
        newGeneration()
        setBase(result.snapshot)
        // The controller returns the canonical (normalized) policy; it
        // replaces the draft only when nothing was edited since sending.
        setWorkspace((current) =>
          settleWorkspaceSave(current, routingBase(result.snapshot), sentSeq, zRoutingPolicy)
        )
        setCompiled(fromSnapshot(result.snapshot))
      }
    })
  }

  const runReload = () => {
    setConfirm(null)
    previewSeq.current += 1
    const generation = ++reloadGeneration.current
    const loader = owner.current
    startReload(async () => {
      const result = await reloadPolicyAction()
      // Superseded, or this editor unmounted: newer work must survive.
      if (generation !== reloadGeneration.current || !drafts.owns(loader)) return
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
      setNodes(result.nodes)
      setWorkspace(openWorkspace(routingBase(result.snapshot), zRoutingPolicy))
      setCompiled(fromSnapshot(result.snapshot))
      setPreview(null)
      setSave(null)
      setHighlight(null)
    })
  }
  const requestReload = () => (unsaved ? setConfirm("reload") : runReload())
  const revertDraft = () => {
    setConfirm(null)
    newGeneration()
    setWorkspace((current) => revertWorkspace(current, zRoutingPolicy))
    setCompiled(fromSnapshot(base))
    setSave(null)
  }

  const globalIssues = issues.filter((issue) => issue.ruleKey === null)
  const ruleIds = new Set(draft.rules.map((rule) => rule.id.trim()))
  const orphanDiagnostics = compiled.diagnostics.filter(
    (diagnostic) => diagnostic.rule_id === null || !ruleIds.has(diagnostic.rule_id)
  )
  const changedRules = states.filter((state) => state.change !== "unchanged").length
  const removed = removedRuleCount(draft, base.policy)
  const defaultChanged = draft.defaultAction !== base.policy.default_action
  const changeSummary = [
    changedRules > 0 ? `${changedRules} 条规则新增或修改` : null,
    removed > 0 ? `删除 ${removed} 条` : null,
    defaultChanged ? "默认动作已改" : null,
    changedRules === 0 && removed === 0 && !defaultChanged && dirty ? "规则顺序已改" : null,
  ].filter((part) => part !== null)

  const editingKey = builder?.session.initial.key ?? null
  const builderRule = editingKey ? draft.rules.find((rule) => rule.key === editingKey) : undefined

  return (
    <div data-testid="policy-editor" className="flex flex-col gap-4">
      <StatusStrip
        base={base}
        activeCount={nodes.filter((node) => node.revoked_at === null).length}
        reloading={reloading}
        disabled={busy}
        onReload={requestReload}
      />

      {!base.valid ? (
        <Alert variant="destructive">
          <TriangleAlertIcon />
          <AlertTitle>已保存的策略当前无效</AlertTitle>
          <AlertDescription className="flex flex-col gap-2">
            <p>
              策略引用了不再可用的节点或存在其他错误，节点收到的生效配置为拒绝全部。原规则保留在下方，修复后保存即可。
            </p>
            <DiagnosticList label="已保存策略的诊断" diagnostics={base.diagnostics} showRule />
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
        <Alert>
          <TriangleAlertIcon />
          <AlertTitle>Controller 已有更新的配置版本 r{observedRevision}</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>
              草稿基于 r{base.revision}，编辑基础不会自动前移，此时保存会因版本冲突被拒绝。
            </p>
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
        <Alert variant="destructive" data-testid="routing-text-invalid">
          <TriangleAlertIcon />
          <AlertTitle>JSONC 文本有 {policyDoc.problems.length} 个问题，图形编辑已暂停</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-2">
            <p>下方是最后一次有效的策略，只读显示。修复文本，或明确放弃文本中尚未生效的修改；切换视图不会丢弃它们。</p>
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={() => setView("jsonc")}>
                回到 JSONC 修复
              </Button>
              <Button variant="ghost" size="sm" onClick={discardText}>
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
                策略 JSONC
                {!textValid ? <Badge variant="destructive">{policyDoc.problems.length} 个问题</Badge> : null}
              </span>
            }
            description="与图形视图是同一份草稿。文本有效时图形视图随之更新；无效时不会被覆盖，也不能保存。"
            actions={<Segmented size="sm" aria-label="编辑方式" value={view} onValueChange={setView} options={viewOptions} />}
          >
            <JsoncPane
              label="路由策略 JSONC"
              text={policyDoc.text}
              problems={textProblems}
              onChange={updateText}
              schemaRoot={routingSchema}
              schema={routingSchema}
              references={{
                node: nodes
                  .filter((node) => node.revoked_at === null)
                  .map((node) => ({ value: node.id, label: node.name, detail: `${node.mesh_ipv4} · ${node.id.slice(0, 8)}` })),
              }}
              readOnly={locked}
              notice={
                change.textOnly ? <>只有注释或排版改动：保存后 controller 会连同原文一起存储，策略内容不变。</> : null
              }
            />
          </Panel>
        ) : (
          <Panel
            title={
              <span className="flex items-center gap-2">
                规则
                <span className="font-mono text-xs font-normal text-muted-foreground tabular-nums">
                  {draft.rules.length}/{POLICY_LIMITS.rules}
                </span>
              </span>
            }
            description="自上而下匹配，第一条命中的规则生效。"
            actions={
              <>
                <Segmented size="sm" aria-label="编辑方式" value={view} onValueChange={setView} options={viewOptions} />
                <Button
                  type="button"
                  size="sm"
                  disabled={visualLocked || draft.rules.length >= POLICY_LIMITS.rules}
                  onClick={newRule}
                >
                  <PlusIcon data-icon="inline-start" />
                  添加规则
                </Button>
              </>
            }
          >
            {draft.rules.length === 0 ? (
              <Empty className="py-10">
                <EmptyHeader>
                  <EmptyTitle>还没有规则</EmptyTitle>
                  <EmptyDescription>
                    所有流量都使用下方的默认动作。添加规则来为特定来源和目标规划路径或拒绝访问。
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
              <RuleList
                rules={draft.rules}
                states={states}
                nodes={nodes}
                highlightKey={highlight}
                disabled={visualLocked}
                actions={{
                  onEdit: editRule,
                  onMove: (index, target) =>
                    updateDraft((current) => ({ ...current, rules: moveItem(current.rules, index, target) })),
                  onDuplicate: duplicateRule,
                  onDelete: (index) => {
                    const rule = draft.rules[index]
                    if (rule) deleteRule(rule.key)
                  },
                }}
              />
            )}

            <div className="flex flex-col gap-3 border-t bg-muted/30 px-3 py-3 sm:flex-row sm:items-center sm:px-4">
              <span className="flex size-6 shrink-0 items-center justify-center rounded-md border border-dashed font-mono text-[0.6875rem] text-muted-foreground">
                ∗
              </span>
              <div className="flex min-w-0 flex-1 flex-col">
                <Label htmlFor={defaultActionId} className="text-sm">
                  默认动作
                </Label>
                <span className="text-xs text-muted-foreground">
                  没有规则命中时使用。直连：Mesh 目标直达，互联网目标由源节点本地出网。
                </span>
              </div>
              <OptionSelect
                id={defaultActionId}
                value={draft.defaultAction}
                options={defaultActionOptions}
                onValueChange={(defaultAction) =>
                  updateDraft((current) => ({ ...current, defaultAction }))
                }
                disabled={visualLocked}
                className={cn("w-32", draft.defaultAction === "direct" && "text-brand")}
              />
            </div>

            {globalIssues.length > 0 || orphanDiagnostics.length > 0 ? (
              <div className="flex flex-col gap-2 border-t px-4 py-3">
                <DiagnosticList label="策略级问题" issues={globalIssues} />
                <DiagnosticList
                  label={compiled.source === "preview" ? "最近一次编译的其他诊断" : "已保存策略的其他诊断"}
                  diagnostics={orphanDiagnostics}
                  stale={!compiledFresh}
                  showRule
                />
              </div>
            ) : null}
          </Panel>
        )}

        <Panel
          className="xl:sticky xl:top-16"
          title={
            <span className="flex items-center gap-2">
              路径预览
              <Badge variant="outline">草稿</Badge>
            </span>
          }
          description="用当前草稿和一条假设流量让 controller 编译决策，不会保存。"
          bodyClassName="flex flex-col gap-4 p-4"
        >
          <FlowForm nodes={nodes} flow={flow} onFlowChange={setFlow} disabled={locked} />
          <PreviewButton
            onClick={runPreview}
            disabled={busy}
            pending={previewing}
            blockedReason={
              !textValid
                ? "先修复 JSONC 文本中的问题。"
                : issues.length > 0
                  ? "先修复草稿中标记的问题。"
                  : flowError
            }
          />
          <div className="border-t pt-4">
            <PreviewOutcome
              result={preview?.result ?? null}
              nodes={nodes}
              flow={preview?.flow ?? flow}
              stale={
                preview !== null &&
                (preview.key !== previewKey ||
                  (preview.result.status === "previewed" &&
                    preview.result.response.revision < base.revision))
              }
              pending={previewing}
              staleMessage={
                preview !== null && preview.key !== previewKey
                  ? "草稿或流量已修改，以下是上一次的结果。"
                  : "以下是保存前的预览结果。"
              }
              emptyMessage="选择源节点和目标，查看这条流量会命中哪条规则、经过哪些节点。规划路径是逻辑意图，不是实际 trace。"
              revisionNote={(response) =>
                // Only a newer controller revision means the base is behind.
                // An older one is just a preview from before the last save.
                response.revision > base.revision
                  ? `编译时 controller 已是 r${response.revision}，草稿基于 r${base.revision}；保存前需要载入最新版本。`
                  : response.revision < base.revision
                    ? `这是 r${response.revision} 时的预览，早于当前编辑基础 r${base.revision}；重新预览以查看最新结果。`
                    : null
              }
              onRuleClick={(ruleId) => {
                const rule = draft.rules.find((candidate) => candidate.id.trim() === ruleId)
                if (rule) setHighlight(rule.key)
              }}
            />
          </div>
        </Panel>
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

      <RuleBuilder
        session={builder?.session ?? null}
        open={builder?.open ?? false}
        dirty={working !== null}
        onWorkingChange={trackBuilderForm}
        onClose={closeBuilder}
        onApply={applyRule}
        onDelete={() => {
          if (editingKey) deleteRule(editingKey)
          closeBuilder()
        }}
        others={draft.rules.filter((rule) => rule.key !== editingKey)}
        defaultAction={draft.defaultAction}
        nodes={nodes}
        diagnostics={builderRule ? diagnosticsForRule(compiled.diagnostics, builderRule.id) : []}
        diagnosticsStale={!compiledFresh}
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
            <AlertDialogTitle>
              {confirm === "revert" ? "放弃草稿中的修改？" : "放弃草稿并重新读取？"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirm === "revert"
                ? `草稿恢复为已保存的 r${base.revision} 策略，编辑基础不变。`
                : "未保存的修改会被丢弃，替换为 controller 上的最新策略和节点清单。"}
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

function StatusStrip({
  base,
  activeCount,
  reloading,
  disabled,
  onReload,
}: {
  base: PolicySnapshot
  activeCount: number
  reloading: boolean
  disabled: boolean
  onReload: () => void
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-xl border bg-card px-4 py-2.5 text-xs">
      <Metric label="编辑基础">
        <span data-testid="policy-base-revision" className="font-mono tabular-nums">
          r{base.revision}
        </span>
      </Metric>
      <Metric label="已保存">
        {base.policy.rules.length} 条规则 · 默认{base.policy.default_action === "reject" ? "拒绝" : "直连"}
      </Metric>
      <Metric label="状态">
        {base.valid ? (
          <Badge variant="outline" className="text-success">有效</Badge>
        ) : (
          <Badge variant="destructive">无效</Badge>
        )}
      </Metric>
      <Metric label="活动节点">
        <span className="tabular-nums">{activeCount}</span>
      </Metric>
      <Metric label="Mesh" className="hidden md:flex">
        <span className="font-mono">{base.network.ipv4_prefix}</span>
        <span className="font-mono text-muted-foreground">{base.network.ipv6_prefix}</span>
      </Metric>
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

function Metric({
  label,
  children,
  className,
}: {
  label: string
  children: ReactNode
  className?: string
}) {
  return (
    <span className={cn("flex items-center gap-1.5", className)}>
      <span className="text-muted-foreground">{label}</span>
      {children}
    </span>
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
  /** Only comments or layout differ from the saved text. */
  textOnly: boolean
  changeSummary: string[]
  issueCount: number
  baseRevision: number
  saving: boolean
  disabled: boolean
  save: SaveResult | null
  onSave: () => void
  onRevert: () => void
  onReload: () => void
}) {
  return (
    <DraftActionBar visible={dirty || textOnly || saving || issueCount > 0 || save !== null}>
      <div data-testid="policy-save-status" aria-live="polite">
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
            已保存为 r{save.snapshot.revision}
            {save.snapshot.valid ? "，节点会在下次同步时拉取。" : "，但策略仍有诊断。"}
          </p>
        ) : null}
      </div>
      <div className="draft-action-bar-row">
        <div className="draft-action-bar-summary text-xs">
          <span
            aria-hidden="true"
            className={cn("size-2 shrink-0 rounded-full", dirty || textOnly ? "bg-warning" : "bg-muted-foreground/30")}
          />
          <span className="min-w-0">
            {dirty ? (
              <>
                <span className="font-medium">未保存的草稿</span>
                <span className="text-muted-foreground"> · {changeSummary.join("，")}</span>
              </>
            ) : textOnly ? (
              <>
                <span className="font-medium">未保存的 JSONC 原文</span>
                <span className="text-muted-foreground"> · 只有注释或排版不同，策略内容与 r{baseRevision} 一致</span>
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
            className={dirty || textOnly || issueCount > 0 ? undefined : "invisible"}
            variant="ghost"
            disabled={disabled || !(dirty || textOnly || issueCount > 0)}
            onClick={onRevert}
          >
            <RotateCcwIcon data-icon="inline-start" />
            放弃修改
          </Button>
          <Button
            size="lg"
            disabled={disabled || !(dirty || textOnly) || issueCount > 0}
            onClick={onSave}
            title={`条件保存：仅当 controller 仍为 r${baseRevision} 时写入`}
          >
            {saving ? (
              <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" />
            ) : (
              <SaveIcon data-icon="inline-start" />
            )}
            保存策略
          </Button>
        </div>
      </div>
    </DraftActionBar>
  )
}
