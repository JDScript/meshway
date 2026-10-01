"use client"

import type { NodeSummary, PolicyDiagnostic } from "@meshway/control-client"
import {
  ArrowDownIcon,
  ArrowUpIcon,
  GlobeIcon,
  InfoIcon,
  PlusIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react"
import { useId, useMemo, useState, type ReactNode } from "react"

import { AdvancedSection } from "@/components/common/advanced-section"
import { OptionSelect } from "@/components/common/option-select"
import { Segmented } from "@/components/common/segmented"
import { DiagnosticList } from "@/components/routing/diagnostics"
import { NodePicker } from "@/components/routing/node-picker"
import { kindLabels, RuleSummary } from "@/components/routing/rule-summary"
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
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import {
  destinationKindOptions,
  nodeRefLabel,
  parseCountryList,
  POLICY_LIMITS,
  resolveNode,
  type DefaultAction,
  type DestinationKind,
  type LocalIssue,
  type RuleDraft,
  type SelectorDraft,
} from "@/lib/policy-model"
import {
  sameForm,
  toRuleDraft,
  validateRuleForm,
  type BuilderSession,
  type RuleForm,
} from "@/lib/rule-form"
import { cn } from "@/lib/utils"

const kindOptions = destinationKindOptions.map(({ value }) => ({ value, label: kindLabels[value] }))

/**
 * Focused editor for one rule. Edits stay in the form until "apply", which
 * hands a RuleDraft back to the policy editor. Unapplied edits (`dirty`) are
 * owned by the editor, which keeps them in the console-scoped draft store:
 * navigating away and back reopens the builder with them. Closing with
 * unapplied edits asks first; while open the sheet is modal, which also keeps
 * the global command menu from navigating away underneath it.
 */
export function RuleBuilder({
  session,
  open,
  dirty,
  onWorkingChange,
  onClose,
  onApply,
  onDelete,
  others,
  defaultAction,
  nodes,
  diagnostics,
  diagnosticsStale,
  disabled,
}: {
  session: BuilderSession | null
  open: boolean
  /** The open session holds unapplied edits. */
  dirty: boolean
  /** Unapplied form contents, or null when the form matches its initial value. */
  onWorkingChange: (form: RuleForm | null) => void
  onClose: () => void
  onApply: (rule: RuleDraft) => void
  onDelete: () => void
  others: RuleDraft[]
  defaultAction: DefaultAction
  nodes: NodeSummary[]
  diagnostics: PolicyDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
}) {
  const [confirmDiscard, setConfirmDiscard] = useState(false)

  const requestClose = () => {
    if (dirty) setConfirmDiscard(true)
    else onClose()
  }

  return (
    <>
      <Sheet
        open={open}
        onOpenChange={(next) => {
          if (!next) requestClose()
        }}
      >
        <SheetContent className="w-full gap-0 data-[side=right]:w-full sm:max-w-xl data-[side=right]:sm:max-w-xl">
          {session ? (
            <BuilderForm
              key={session.id}
              session={session}
              others={others}
              defaultAction={defaultAction}
              nodes={nodes}
              diagnostics={diagnostics}
              diagnosticsStale={diagnosticsStale}
              disabled={disabled}
              onWorkingChange={onWorkingChange}
              onCancel={requestClose}
              onApply={onApply}
              onDelete={onDelete}
            />
          ) : null}
        </SheetContent>
      </Sheet>
      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>放弃对这条规则的修改？</AlertDialogTitle>
            <AlertDialogDescription>
              规则表单中的修改尚未应用到草稿。策略草稿本身不受影响。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() => {
                setConfirmDiscard(false)
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

function BuilderForm({
  session,
  others,
  defaultAction,
  nodes,
  diagnostics,
  diagnosticsStale,
  disabled,
  onWorkingChange,
  onCancel,
  onApply,
  onDelete,
}: {
  session: BuilderSession
  others: RuleDraft[]
  defaultAction: DefaultAction
  nodes: NodeSummary[]
  diagnostics: PolicyDiagnostic[]
  diagnosticsStale: boolean
  disabled: boolean
  onWorkingChange: (form: RuleForm | null) => void
  onCancel: () => void
  onApply: (rule: RuleDraft) => void
  onDelete: () => void
}) {
  const [form, setFormState] = useState<RuleForm>(session.working ?? session.initial)
  const [attempted, setAttempted] = useState(false)
  const baseId = useId()
  // Unique among the current rows, including rows restored from the store.
  const newKey = () => {
    const taken = new Set(form.destinations.map((selector) => selector.key))
    let n = 0
    while (taken.has(`${form.key}-f${n}`)) n += 1
    return `${form.key}-f${n}`
  }

  const setForm = (next: RuleForm) => {
    setFormState(next)
    onWorkingChange(sameForm(next, session.initial) ? null : next)
  }

  const issues = useMemo(
    () => validateRuleForm(form, others, defaultAction),
    [form, others, defaultAction]
  )
  const draft = toRuleDraft(form)
  const has = (prefix: string) =>
    issues.some((issue) => issue.field.startsWith(prefix)) ||
    diagnostics.some((diagnostic) => diagnostic.field.startsWith(prefix))
  const show = (field: string) => (attempted || session.mode === "edit") && has(field)

  const countries = parseCountryList(form.sourceCountries)
  const selectorCount =
    (form.sourceMode === "specific" ? form.sourceNodeIds.length + countries.length : 0) +
    (form.targetMode === "specific" ? form.destinations.length : 0)
  const selectorsFull = selectorCount >= POLICY_LIMITS.selectorsPerRule

  const activeNodes = nodes.filter((node) => node.revoked_at === null)
  const staleSources = form.sourceNodeIds.filter(
    (id) => !activeNodes.some((node) => node.id === id)
  )

  const setDestination = (position: number, selector: SelectorDraft) =>
    setForm({
      ...form,
      destinations: form.destinations.map((existing, i) => (i === position ? selector : existing)),
    })
  const moveVia = (position: number, offset: -1 | 1) => {
    const via = [...form.via]
    const target = position + offset
    const [moved] = via.splice(position, 1)
    if (moved === undefined || target < 0 || target > via.length) return
    via.splice(target, 0, moved)
    setForm({ ...form, via })
  }

  const route = form.action === "route"
  const exitAllowed = form.scope === "internet"
  const showExit = route && (exitAllowed || form.exitNodeId !== "")
  const hints: string[] = []
  if (route && form.exitNodeId !== "" && form.scope === "mesh") {
    hints.push("Mesh 目标本身就是终点，不能设置出口；请清除出口节点。")
  } else if (route && form.exitNodeId !== "" && form.scope === "any") {
    hints.push("出口节点只适用于「互联网」范围。")
  }
  if (route && form.via.length > 0 && form.exitNodeId === "" && form.scope !== "mesh") {
    hints.push("经由节点访问互联网时必须指定出口；「任意」范围请改为 Mesh 节点或互联网。")
  }

  const apply = () => {
    setAttempted(true)
    if (issues.length > 0) return
    onApply(draft)
  }

  return (
    <form
      className="flex min-h-0 flex-1 flex-col"
      onSubmit={(event) => {
        event.preventDefault()
        apply()
      }}
    >
      <SheetHeader className="gap-1 border-b p-4 pr-12">
        <SheetTitle className="text-base">
          {session.mode === "new" ? "新建规则" : "编辑规则"}
        </SheetTitle>
        <SheetDescription>
          第 {session.position + 1} 条 · 自上而下匹配，第一条命中的规则生效。同一组内的条件满足任一即可。
        </SheetDescription>
      </SheetHeader>

      <div className="flex-1 overflow-y-auto">
        <div className="flex flex-col gap-2 border-b bg-muted/30 px-4 py-3">
          <RuleSummary rule={draft} nodes={nodes} />
          {form.sourceMode === "any" && form.targetMode === "all" ? (
            <p className="flex items-start gap-1.5 text-xs text-warning">
              <InfoIcon className="mt-0.5 size-3.5 shrink-0" />
              {form.scope === "any"
                ? "这条规则匹配全部流量：排在它后面的规则和默认动作都不会再生效。"
                : `这条规则匹配所有来源访问的全部${form.scope === "mesh" ? " Mesh 节点" : "互联网"}目标，后面同范围的规则不会再生效。`}
            </p>
          ) : null}
        </div>

        <div className="flex flex-col divide-y">
          <Section title="来源" description="节点与国家同时填写时需同时满足；同一类内满足任一即可。">
            <Segmented
              value={form.sourceMode}
              options={[
                { value: "specific", label: "指定来源" },
                { value: "any", label: "任意来源" },
              ]}
              onValueChange={(sourceMode) => setForm({ ...form, sourceMode })}
              aria-label="来源范围"
            />
            {form.sourceMode === "specific" ? (
              <div className="grid gap-3">
                <div className="flex flex-col gap-1.5">
                  <span id={`${baseId}-sources`} className="text-xs font-medium">
                    源节点
                  </span>
                  {activeNodes.length === 0 && staleSources.length === 0 ? (
                    <span className="text-xs text-muted-foreground">没有可选的活动节点。</span>
                  ) : (
                    <div
                      role="group"
                      aria-labelledby={`${baseId}-sources`}
                      className="grid max-h-44 gap-0.5 overflow-y-auto rounded-lg border p-1 sm:grid-cols-2"
                    >
                      {[...activeNodes.map((node) => node.id), ...staleSources].map((id) => {
                        const ref = resolveNode(nodes, id)
                        const checked = form.sourceNodeIds.includes(id)
                        return (
                          <label
                            key={id}
                            className={cn(
                              "flex min-w-0 cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted",
                              ref.kind !== "active" && "text-destructive"
                            )}
                          >
                            <Checkbox
                              checked={checked}
                              disabled={!checked && selectorsFull}
                              onCheckedChange={(next) =>
                                setForm({
                                  ...form,
                                  sourceNodeIds: next
                                    ? [...form.sourceNodeIds, id]
                                    : form.sourceNodeIds.filter((existing) => existing !== id),
                                })
                              }
                            />
                            <span className="truncate">{nodeRefLabel(ref)}</span>
                            {ref.kind === "active" ? (
                              <span className="ml-auto font-mono text-[0.625rem] text-muted-foreground">
                                {ref.node.country ?? "—"}
                              </span>
                            ) : null}
                          </label>
                        )
                      })}
                    </div>
                  )}
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor={`${baseId}-countries`}>源国家</Label>
                  <Input
                    id={`${baseId}-countries`}
                    value={form.sourceCountries}
                    onChange={(event) => setForm({ ...form, sourceCountries: event.target.value })}
                    placeholder="例如 CN, US"
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={show("source.countries") || undefined}
                    className="max-w-xs font-mono"
                  />
                  <span className="text-xs text-muted-foreground">
                    两位字母，逗号分隔；匹配管理员为节点标注的国家。
                  </span>
                </div>
              </div>
            ) : (
              <p className="text-xs text-muted-foreground">匹配所有来源节点，包括未设置国家的节点。</p>
            )}
            <FieldIssues issues={issues} field="source" visible={attempted} />
          </Section>

          <Section title="目标">
            <div className="grid gap-3">
              <div className="flex flex-col gap-1.5">
                <span id={`${baseId}-scope`} className="text-xs font-medium">
                  目标范围
                </span>
                <Segmented
                  value={form.scope}
                  options={[
                    { value: "internet", label: "互联网" },
                    { value: "mesh", label: "Mesh 节点" },
                    { value: "any", label: "任意" },
                  ]}
                  onValueChange={(scope) => setForm({ ...form, scope })}
                  aria-labelledby={`${baseId}-scope`}
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <span id={`${baseId}-match`} className="text-xs font-medium">
                  匹配
                </span>
                <Segmented
                  value={form.targetMode}
                  options={[
                    { value: "specific", label: "指定条件（任一）" },
                    { value: "all", label: "范围内全部目标" },
                  ]}
                  onValueChange={(targetMode) =>
                    setForm({
                      ...form,
                      targetMode,
                      destinations:
                        targetMode === "specific" && form.destinations.length === 0
                          ? [
                              {
                                key: newKey(),
                                kind: form.scope === "mesh" ? "node" : "domain_suffix",
                                value: "",
                              },
                            ]
                          : form.destinations,
                    })
                  }
                  aria-labelledby={`${baseId}-match`}
                />
              </div>
              {form.targetMode === "specific" ? (
                <div className="flex flex-col gap-2">
                  <ol className="flex flex-col gap-2">
                    {form.destinations.map((selector, position) => (
                      <SelectorRow
                        key={selector.key}
                        baseId={baseId}
                        position={position}
                        selector={selector}
                        nodes={nodes}
                        invalid={show(`destination_any[${position}]`)}
                        onChange={(next) => setDestination(position, next)}
                        onRemove={() =>
                          setForm({
                            ...form,
                            destinations: form.destinations.filter((_, i) => i !== position),
                          })
                        }
                      />
                    ))}
                  </ol>
                  <div className="flex flex-wrap items-center gap-3">
                    <Button
                      type="button"
                      variant="outline"
                      disabled={selectorsFull}
                      onClick={() =>
                        setForm({
                          ...form,
                          destinations: [
                            ...form.destinations,
                            {
                              key: newKey(),
                              kind: form.scope === "mesh" ? "node" : "domain_suffix",
                              value: "",
                            },
                          ],
                        })
                      }
                    >
                      <PlusIcon data-icon="inline-start" />
                      添加条件
                    </Button>
                    <span className="text-[0.6875rem] text-muted-foreground tabular-nums">
                      条件合计 {selectorCount}/{POLICY_LIMITS.selectorsPerRule}
                    </span>
                  </div>
                  <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
                    域名后缀按标签边界匹配自身与子域（example.com 不匹配 badexample.com）；CIDR 支持 IPv4 / IPv6；外部目标国家仅在预览时手动提供。
                  </p>
                </div>
              ) : (
                <p className="text-xs text-muted-foreground">
                  匹配「{form.scope === "any" ? "任意" : form.scope === "mesh" ? "Mesh 节点" : "互联网"}」范围内的所有目标。
                </p>
              )}
              <FieldIssues issues={issues} field="destination_any" visible={attempted} />
            </div>
          </Section>

          <Section title="动作">
            <Segmented
              value={form.action}
              options={[
                { value: "route", label: "路由" },
                { value: "reject", label: "拒绝" },
              ]}
              onValueChange={(action) => setForm({ ...form, action })}
              aria-label="动作"
            />
            {route ? (
              <div className="grid gap-3">
                <PathBuilder
                  baseId={baseId}
                  form={form}
                  nodes={nodes}
                  show={show}
                  onChange={setForm}
                  onMove={moveVia}
                />
                {showExit ? (
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor={`${baseId}-exit`}>出口节点</Label>
                    <NodePicker
                      id={`${baseId}-exit`}
                      nodes={nodes}
                      value={form.exitNodeId}
                      exclude={form.via}
                      onValueChange={(exitNodeId) => setForm({ ...form, exitNodeId })}
                      placeholder="选择出口"
                      noneLabel="无（源节点本地出网）"
                      invalid={show("action.exit_node_id")}
                      className="w-full max-w-xs"
                    />
                    <span className="text-xs text-muted-foreground">
                      流量从该节点离开 mesh 访问互联网。
                    </span>
                  </div>
                ) : null}
                {hints.map((hint) => (
                  <p key={hint} className="flex items-start gap-1.5 text-xs text-warning">
                    <InfoIcon className="mt-0.5 size-3.5 shrink-0" />
                    {hint}
                  </p>
                ))}
              </div>
            ) : (
              <p className="text-xs text-muted-foreground">
                命中的流量被拒绝，不再继续匹配后续规则。
                {form.via.length > 0 || form.exitNodeId !== ""
                  ? form.initialAction === "route"
                    ? "应用后会清除经由与出口节点；切回路由可恢复。"
                    : "这条拒绝规则仍保存着经由或出口节点，编译会报告错误；切换到「路由」可查看并移除它们。"
                  : null}
              </p>
            )}
          </Section>

          <AdvancedSection summary={`ID ${form.id.trim() || "—"}`} attention={show("id")}>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor={`${baseId}-id`}>规则 ID</Label>
              <Input
                id={`${baseId}-id`}
                value={form.id}
                onChange={(event) => setForm({ ...form, id: event.target.value })}
                autoComplete="off"
                spellCheck={false}
                aria-invalid={show("id") || undefined}
                className="max-w-xs font-mono"
              />
              <FieldIssues issues={issues} field="id" visible={attempted || session.mode === "edit"} />
              <span className="text-xs text-muted-foreground">在策略中唯一，诊断与预览结果用它指代这条规则。</span>
            </div>
          </AdvancedSection>

          {issues.length > 0 && attempted ? (
            <Section title="需要修复">
              <DiagnosticList label="规则表单问题" issues={issues} />
            </Section>
          ) : null}
          {diagnostics.length > 0 ? (
            <Section title={diagnosticsStale ? "上次编译的诊断" : "编译诊断"}>
              <DiagnosticList
                label="规则的编译诊断"
                diagnostics={diagnostics}
                stale={diagnosticsStale}
              />
            </Section>
          ) : null}
        </div>
      </div>

      <SheetFooter className="flex-row items-center gap-2 border-t p-3">
        {session.mode === "edit" ? (
          <Button type="button" variant="ghost" className="text-destructive" onClick={onDelete} disabled={disabled}>
            <Trash2Icon data-icon="inline-start" />
            删除规则
          </Button>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          <Button type="button" variant="outline" onClick={onCancel}>
            取消
          </Button>
          <Button type="submit" disabled={disabled}>
            {session.mode === "new" ? "添加到草稿" : "应用到草稿"}
          </Button>
        </div>
      </SheetFooter>
    </form>
  )
}

function Section({
  title,
  description,
  children,
}: {
  title: string
  description?: string
  children: ReactNode
}) {
  return (
    <section className="flex flex-col gap-3 px-4 py-4">
      <div className="flex flex-col gap-0.5">
        <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h3>
        {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
      </div>
      {children}
    </section>
  )
}

function FieldIssues({
  issues,
  field,
  visible,
}: {
  issues: LocalIssue[]
  field: string
  visible: boolean
}) {
  const matching = issues.filter((issue) => issue.field === field)
  if (!visible || matching.length === 0) return null
  return (
    <p role="alert" className="text-xs text-destructive">
      {matching.map((issue) => issue.message).join(" ")}
    </p>
  )
}

function SelectorRow({
  baseId,
  position,
  selector,
  nodes,
  invalid,
  onChange,
  onRemove,
}: {
  baseId: string
  position: number
  selector: SelectorDraft
  nodes: NodeSummary[]
  invalid: boolean
  onChange: (selector: SelectorDraft) => void
  onRemove: () => void
}) {
  const option = destinationKindOptions.find(({ value }) => value === selector.kind)
  const valueId = `${baseId}-value-${selector.key}`
  return (
    <li className="flex items-center gap-2">
      <OptionSelect<DestinationKind>
        value={selector.kind}
        options={kindOptions}
        // Values of one kind are meaningless for another.
        onValueChange={(kind) => {
          if (kind !== selector.kind) onChange({ ...selector, kind, value: "" })
        }}
        aria-label={`条件 ${position + 1} 类型`}
        className="w-26 shrink-0"
      />
      <div className="min-w-0 flex-1">
        <Label htmlFor={valueId} className="sr-only">
          条件 {position + 1} 值
        </Label>
        {selector.kind === "node" ? (
          <NodePicker
            id={valueId}
            nodes={nodes}
            value={selector.value}
            onValueChange={(value) => onChange({ ...selector, value })}
            placeholder="选择节点"
            invalid={invalid}
            className="w-full"
          />
        ) : (
          <Input
            id={valueId}
            value={selector.value}
            onChange={(event) => onChange({ ...selector, value: event.target.value })}
            placeholder={option?.placeholder}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={invalid || undefined}
            className="font-mono"
          />
        )}
      </div>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label={`移除目标条件 ${position + 1}`}
        onClick={onRemove}
      >
        <XIcon />
      </Button>
    </li>
  )
}

function PathBuilder({
  baseId,
  form,
  nodes,
  show,
  onChange,
  onMove,
}: {
  baseId: string
  form: RuleForm
  nodes: NodeSummary[]
  show: (field: string) => boolean
  onChange: (form: RuleForm) => void
  onMove: (position: number, offset: -1 | 1) => void
}) {
  return (
    <div className="flex flex-col gap-2">
      <span className="text-xs font-medium">经由节点（按顺序必经）</span>
      <ol className="flex flex-col">
        <PathEndpoint label="来源节点" />
        {form.via.map((nodeId, position) => {
          const viaId = `${baseId}-via-${position}`
          return (
            <li key={position} className="relative flex items-center gap-2 py-1 pl-6">
              <span aria-hidden="true" className="absolute top-0 bottom-0 left-2 border-l border-dashed border-brand/50" />
              <span
                aria-hidden="true"
                className="absolute left-0.5 flex size-3.5 items-center justify-center rounded-full bg-muted font-mono text-[0.5625rem] text-muted-foreground"
              >
                {position + 1}
              </span>
              <Label htmlFor={viaId} className="sr-only">
                经由节点 {position + 1}
              </Label>
              <NodePicker
                id={viaId}
                nodes={nodes}
                value={nodeId}
                exclude={form.via}
                onValueChange={(value) =>
                  onChange({
                    ...form,
                    via: form.via.map((existing, i) => (i === position ? value : existing)),
                  })
                }
                placeholder="选择节点"
                invalid={show(`action.via[${position}]`)}
                className="min-w-0 flex-1"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={`经由节点 ${position + 1} 上移`}
                disabled={position === 0}
                onClick={() => onMove(position, -1)}
              >
                <ArrowUpIcon />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={`经由节点 ${position + 1} 下移`}
                disabled={position === form.via.length - 1}
                onClick={() => onMove(position, 1)}
              >
                <ArrowDownIcon />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={`移除经由节点 ${position + 1}`}
                onClick={() => onChange({ ...form, via: form.via.filter((_, i) => i !== position) })}
              >
                <XIcon />
              </Button>
            </li>
          )
        })}
        <li className="relative py-1 pl-6">
          <span aria-hidden="true" className="absolute top-0 bottom-1/2 left-2 border-l border-dashed border-brand/50" />
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={form.via.length >= POLICY_LIMITS.via}
            onClick={() => onChange({ ...form, via: [...form.via, ""] })}
          >
            <PlusIcon data-icon="inline-start" />
            添加经由节点（{form.via.length}/{POLICY_LIMITS.via}）
          </Button>
        </li>
        <PathEndpoint
          label={
            form.scope === "mesh"
              ? "目标节点"
              : form.exitNodeId
                ? "出口 → 互联网目标"
                : form.scope === "internet"
                  ? "互联网目标（源节点本地出网）"
                  : "目标"
          }
          external={form.scope !== "mesh"}
        />
      </ol>
      <span className="text-xs text-muted-foreground">
        经由节点是有序的逻辑必经节点，不会为了可达性被绕过。
      </span>
    </div>
  )
}

function PathEndpoint({ label, external = false }: { label: string; external?: boolean }) {
  return (
    <li className="flex items-center gap-2 py-1 text-xs text-muted-foreground">
      <span
        aria-hidden="true"
        className="flex size-4 items-center justify-center rounded-full border border-brand/60 bg-background"
      >
        {external ? <GlobeIcon className="size-2.5 text-brand" /> : <span className="size-1.5 rounded-full bg-brand" />}
      </span>
      {label}
    </li>
  )
}
