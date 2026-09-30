"use client"

import type {
  NodeSummary,
  PolicyDiagnostic,
  PolicySnapshot,
} from "@meshway/control-client"
import {
  CircleAlertIcon,
  PlusIcon,
  RefreshCwIcon,
  SaveIcon,
  TriangleAlertIcon,
} from "lucide-react"
import { useEffect, useId, useMemo, useRef, useState, useTransition } from "react"

import { DiagnosticList } from "@/components/routing/diagnostics"
import { RoutePreview } from "@/components/routing/route-preview"
import { RuleCard } from "@/components/routing/rule-card"
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
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import type { PreviewResult, SaveResult } from "@/lib/action-state"
import {
  previewPolicyAction,
  reloadPolicyAction,
  savePolicyAction,
} from "@/lib/policy-actions"
import {
  defaultActionOptions,
  diagnosticsForRule,
  emptyFlow,
  fromPolicy,
  parseDefaultAction,
  POLICY_LIMITS,
  samePolicy,
  toFlow,
  toPolicy,
  validateDraft,
  validateFlow,
  type FlowDraft,
  type PolicyDraft,
  type RuleDraft,
} from "@/lib/policy-model"

type Compiled = {
  source: "saved" | "preview"
  valid: boolean
  diagnostics: PolicyDiagnostic[]
}

const fromSnapshot = (snapshot: PolicySnapshot): Compiled => ({
  source: "saved",
  valid: snapshot.valid,
  diagnostics: snapshot.diagnostics,
})

function uniqueRuleId(rules: RuleDraft[]): string {
  const taken = new Set(rules.map((rule) => rule.id.trim()))
  let n = rules.length + 1
  while (taken.has(`rule-${n}`)) n += 1
  return `rule-${n}`
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
}: {
  snapshot: PolicySnapshot
  nodes: NodeSummary[]
}) {
  const [base, setBase] = useState(snapshot)
  const [nodes, setNodes] = useState(initialNodes)
  const [draft, setDraft] = useState<PolicyDraft>(() => fromPolicy(snapshot.policy))
  const [compiled, setCompiled] = useState<Compiled>(() => fromSnapshot(snapshot))
  const [flow, setFlow] = useState<FlowDraft>(emptyFlow)
  const [preview, setPreview] = useState<{ result: PreviewResult; key: string } | null>(null)
  const [save, setSave] = useState<SaveResult | null>(null)
  const [reloadError, setReloadError] = useState<string | null>(null)
  const [confirmReload, setConfirmReload] = useState(false)
  const [previewing, startPreview] = useTransition()
  const [saving, startSave] = useTransition()
  const [reloading, startReload] = useTransition()
  const nextKey = useRef(0)
  const defaultActionId = useId()

  const policy = useMemo(() => toPolicy(draft), [draft])
  const issues = useMemo(() => validateDraft(draft), [draft])
  const dirty = !samePolicy(policy, base.policy)
  const busy = previewing || saving || reloading
  const previewKey = JSON.stringify([policy, toFlow(flow)])
  const flowError = validateFlow(flow)

  const previewRevision =
    preview?.result.status === "previewed" ? preview.result.response.revision : 0
  const observedRevision = Math.max(snapshot.revision, previewRevision)
  const behind = observedRevision > base.revision

  useEffect(() => {
    if (!dirty) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [dirty])

  const updateRules = (rules: RuleDraft[]) => {
    setDraft({ ...draft, rules })
    setSave(null)
  }
  const moveRule = (index: number, offset: -1 | 1) => {
    const rules = [...draft.rules]
    const [moved] = rules.splice(index, 1)
    const target = index + offset
    if (moved === undefined || target < 0 || target > rules.length) return
    rules.splice(target, 0, moved)
    updateRules(rules)
  }
  const addRule = () =>
    updateRules([
      ...draft.rules,
      {
        key: `new-${nextKey.current++}`,
        id: uniqueRuleId(draft.rules),
        sourceNodeIds: [],
        sourceCountries: "",
        scope: "internet",
        destinations: [],
        action: "route",
        via: [],
        exitNodeId: "",
      },
    ])

  const runPreview = () => {
    const key = previewKey
    const body = policy
    const query = toFlow(flow)
    startPreview(async () => {
      const result = await previewPolicyAction(body, query)
      setPreview({ result, key })
      if (result.status === "previewed") {
        setCompiled({
          source: "preview",
          valid: result.response.valid,
          diagnostics: result.response.diagnostics,
        })
      }
    })
  }

  const runSave = () => {
    const expected = base.revision
    const body = policy
    startSave(async () => {
      const result = await savePolicyAction(expected, body)
      setSave(result)
      if (result.status === "saved") {
        setBase(result.snapshot)
        // The controller returns the canonical (normalized) policy.
        setDraft(fromPolicy(result.snapshot.policy))
        setCompiled(fromSnapshot(result.snapshot))
      }
    })
  }

  const runReload = () => {
    setConfirmReload(false)
    startReload(async () => {
      const result = await reloadPolicyAction()
      if (result.status === "error") {
        setReloadError(result.message)
        return
      }
      setReloadError(null)
      setBase(result.snapshot)
      setNodes(result.nodes)
      setDraft(fromPolicy(result.snapshot.policy))
      setCompiled(fromSnapshot(result.snapshot))
      setPreview(null)
      setSave(null)
    })
  }
  const requestReload = () => (dirty ? setConfirmReload(true) : runReload())

  const blockedReason =
    issues.length > 0
      ? "修复草稿中标记的问题后才能编译。"
      : flowError
  const globalIssues = issues.filter((issue) => issue.ruleKey === null)
  const ruleIds = new Set(draft.rules.map((rule) => rule.id.trim()))
  const orphanDiagnostics = compiled.diagnostics.filter(
    (diagnostic) => diagnostic.rule_id === null || !ruleIds.has(diagnostic.rule_id)
  )
  const activeCount = nodes.filter((node) => node.revoked_at === null).length

  return (
    <div data-testid="policy-editor" className="flex flex-col gap-6">
      <Card className="[--card-spacing:--spacing(5)]">
        <CardHeader>
          <CardTitle className="text-base">策略状态</CardTitle>
          <CardDescription className="text-sm">
            版本号在节点清单、节点国家或策略变化时递增；心跳和配置确认不改变版本。
          </CardDescription>
          <CardAction>
            <Button
              type="button"
              variant="outline"
              size="lg"
              disabled={busy}
              onClick={requestReload}
            >
              <RefreshCwIcon data-icon="inline-start" className={reloading ? "animate-spin" : undefined} />
              {reloading ? "正在读取" : "重新读取"}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 text-sm">
            <dt className="text-muted-foreground">编辑基础版本</dt>
            <dd data-testid="policy-base-revision" className="font-mono text-xs leading-5 tabular-nums">
              {base.revision}
            </dd>
            <dt className="text-muted-foreground">Mesh IPv4 段</dt>
            <dd className="font-mono text-xs leading-5">{base.network.ipv4_prefix}</dd>
            <dt className="text-muted-foreground">Mesh IPv6 段</dt>
            <dd className="font-mono text-xs leading-5">{base.network.ipv6_prefix}</dd>
            <dt className="text-muted-foreground">已保存默认动作</dt>
            <dd className="text-xs leading-5">
              {defaultActionOptions.find(({ value }) => value === base.policy.default_action)?.label}
            </dd>
            <dt className="text-muted-foreground">已保存规则</dt>
            <dd className="font-mono text-xs leading-5 tabular-nums">{base.policy.rules.length}</dd>
            <dt className="text-muted-foreground">活动节点</dt>
            <dd className="font-mono text-xs leading-5 tabular-nums">{activeCount}</dd>
            <dt className="text-muted-foreground">草稿</dt>
            <dd className="text-xs leading-5">
              {dirty ? <Badge variant="secondary">有未保存的修改</Badge> : "与已保存策略一致"}
            </dd>
          </dl>

          {!base.valid ? (
            <Alert variant="destructive" className="px-4 py-3 text-sm">
              <TriangleAlertIcon />
              <AlertTitle className="text-sm">已保存的策略当前无效</AlertTitle>
              <AlertDescription className="flex flex-col gap-2 text-sm">
                <p>
                  已保存的策略引用了不再可用的节点或存在其他错误。修复之前，controller
                  向节点下发的生效配置为拒绝全部（deny-all）。规则仍可编辑，修复后保存即可。
                </p>
                <DiagnosticList label="已保存策略的诊断" diagnostics={base.diagnostics} showRule />
              </AlertDescription>
            </Alert>
          ) : null}

          {behind ? (
            <Alert className="px-4 py-3 text-sm">
              <TriangleAlertIcon />
              <AlertTitle className="text-sm">
                Controller 已有更新的版本 {observedRevision}
              </AlertTitle>
              <AlertDescription className="text-sm">
                草稿基于版本 {base.revision}。编辑基础不会自动前移；此时保存会被拒绝，以免覆盖其他修改。
                需要时请重新读取{dirty ? "（会先确认放弃当前草稿）" : ""}。
              </AlertDescription>
            </Alert>
          ) : null}
          {reloadError ? (
            <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
              <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
              <span className="break-all">{reloadError}</span>
            </p>
          ) : null}
        </CardContent>
      </Card>

      <fieldset disabled={saving || reloading} className="flex min-w-0 flex-col gap-6">
        <Card className="[--card-spacing:--spacing(5)]">
          <CardHeader>
            <CardTitle className="text-base">
              规则
              <span className="ml-2 font-mono text-sm font-normal text-muted-foreground tabular-nums">
                {draft.rules.length}/{POLICY_LIMITS.rules}
              </span>
            </CardTitle>
            <CardDescription className="text-sm">
              从上到下依次匹配，第一条命中的规则生效；都不命中时使用默认动作。策略只描述逻辑意图，不授予访问权限，也不代表链路可达。
            </CardDescription>
            <CardAction>
              <Button
                type="button"
                variant="outline"
                size="lg"
                disabled={draft.rules.length >= POLICY_LIMITS.rules}
                onClick={addRule}
              >
                <PlusIcon data-icon="inline-start" />
                添加规则
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            {draft.rules.length === 0 ? (
              <p className="rounded-md border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">
                没有规则。所有流量都使用默认动作。
              </p>
            ) : (
              <ol className="flex flex-col gap-4">
                {draft.rules.map((rule, index) => (
                  <RuleCard
                    key={rule.key}
                    rule={rule}
                    index={index}
                    count={draft.rules.length}
                    nodes={nodes}
                    issues={issues.filter((issue) => issue.ruleKey === rule.key)}
                    diagnostics={diagnosticsForRule(compiled.diagnostics, rule.id)}
                    onChange={(next) =>
                      updateRules(draft.rules.map((existing, i) => (i === index ? next : existing)))
                    }
                    onMove={(offset) => moveRule(index, offset)}
                    onRemove={() => updateRules(draft.rules.filter((_, i) => i !== index))}
                  />
                ))}
              </ol>
            )}

            <div className="flex flex-col gap-2 border-t pt-4">
              <Label htmlFor={defaultActionId}>默认动作</Label>
              <NativeSelect
                id={defaultActionId}
                value={draft.defaultAction}
                onChange={(event) => {
                  const defaultAction = parseDefaultAction(event.target.value)
                  if (defaultAction) {
                    setDraft({ ...draft, defaultAction })
                    setSave(null)
                  }
                }}
                className="w-48"
              >
                {defaultActionOptions.map((option) => (
                  <NativeSelectOption key={option.value} value={option.value}>
                    {option.label}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
              <span className="text-xs text-muted-foreground">
                没有规则命中时使用。本地直连表示由源节点自己访问目标，不经过其他节点。
              </span>
            </div>

            <DiagnosticList label="策略级问题" issues={globalIssues} />
            {compiled.diagnostics.length > 0 ? (
              <DiagnosticList
                label={compiled.source === "preview" ? "最近一次编译的其他诊断" : "已保存策略的其他诊断"}
                diagnostics={orphanDiagnostics}
                showRule
              />
            ) : null}
          </CardContent>
        </Card>
      </fieldset>

      <RoutePreview
        nodes={nodes}
        flow={flow}
        onFlowChange={setFlow}
        result={preview?.result ?? null}
        stale={preview !== null && preview.key !== previewKey}
        pending={previewing}
        disabled={busy}
        blockedReason={blockedReason}
        onPreview={runPreview}
        baseRevision={base.revision}
      />

      <Card className="[--card-spacing:--spacing(5)]">
        <CardHeader>
          <CardTitle className="text-base">保存</CardTitle>
          <CardDescription className="text-sm">
            条件保存：仅当 controller 的配置版本仍为 {base.revision} 时写入。无效策略不会被保存，也不会丢弃任何规则。保存后节点会拉取新配置；节点「已校验保存」不代表网络已生效，数据面转发尚未实现。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              size="lg"
              disabled={busy || issues.length > 0}
              onClick={runSave}
            >
              <SaveIcon data-icon="inline-start" />
              {saving ? "正在保存" : "保存策略"}
            </Button>
            {issues.length > 0 ? (
              <span className="text-xs text-muted-foreground">
                修复草稿中标记的问题后才能保存。
              </span>
            ) : null}
          </div>
          <div data-testid="policy-save-status" aria-live="polite">
            {save?.status === "saved" ? (
              <p role="status" className="text-xs text-muted-foreground">
                已保存。当前版本 {save.snapshot.revision}
                {save.snapshot.valid ? "。" : "，但策略仍有诊断，请查看上方。"}
              </p>
            ) : save?.status === "conflict" ? (
              <Alert variant="destructive" className="px-4 py-3 text-sm">
                <TriangleAlertIcon />
                <AlertTitle className="text-sm">版本冲突，未保存</AlertTitle>
                <AlertDescription className="flex flex-col gap-2 text-sm">
                  <p>{save.message}</p>
                  <p>
                    需要时先记下你的修改，再载入最新版本重新编辑；载入会放弃当前草稿。
                  </p>
                  <div>
                    <Button
                      type="button"
                      variant="outline"
                      disabled={busy}
                      onClick={requestReload}
                    >
                      载入最新版本
                    </Button>
                  </div>
                </AlertDescription>
              </Alert>
            ) : save?.status === "invalid" || save?.status === "error" ? (
              <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
                <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
                <span className="break-all">{save.message}</span>
              </p>
            ) : null}
          </div>
        </CardContent>
      </Card>

      <AlertDialog open={confirmReload} onOpenChange={setConfirmReload}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>放弃草稿并重新读取？</AlertDialogTitle>
            <AlertDialogDescription>
              编辑器中未保存的修改会被丢弃，替换为 controller 上的最新策略和节点清单。此操作不能撤销。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>继续编辑</AlertDialogCancel>
            <Button type="button" variant="destructive" onClick={runReload}>
              放弃草稿并重新读取
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
