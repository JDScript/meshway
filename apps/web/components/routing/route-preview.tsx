"use client"

import type { NodeSummary, PreviewPolicyResponse } from "@meshway/control-client"
import { CircleAlertIcon, HistoryIcon, PlayIcon } from "lucide-react"
import { useId } from "react"

import { DiagnosticList } from "@/components/routing/diagnostics"
import { NodePicker } from "@/components/routing/node-picker"
import { PathTrace, planSteps } from "@/components/routing/path-trace"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Spinner } from "@/components/ui/spinner"
import type { PreviewResult } from "@/lib/action-state"
import type { FlowDraft } from "@/lib/policy-model"

const decisionBadge = {
  route: { label: "路由", variant: "default" },
  reject: { label: "拒绝", variant: "destructive" },
  invalid: { label: "无效", variant: "destructive" },
} as const

export function externalLabel(flow: FlowDraft): string | null {
  if (flow.targetNodeId !== "") return null
  return flow.domain.trim() || flow.ip.trim() || null
}

/**
 * Asks the controller to compile a policy against one hypothetical flow and
 * shows its decision. Shared by the draft editor and the logical map.
 */
export function FlowForm({
  nodes,
  flow,
  onFlowChange,
  disabled,
}: {
  nodes: NodeSummary[]
  flow: FlowDraft
  onFlowChange: (flow: FlowDraft) => void
  disabled?: boolean
}) {
  const id = useId()
  const external = flow.targetNodeId === ""
  const active = nodes.filter((node) => node.revoked_at === null)

  return (
    <div className="grid gap-3">
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${id}-source`}>源节点</Label>
          <NodePicker
            id={`${id}-source`}
            data-testid="preview-source"
            nodes={active}
            value={flow.sourceNodeId}
            onValueChange={(sourceNodeId) => onFlowChange({ ...flow, sourceNodeId })}
            placeholder="选择源节点"
            disabled={disabled}
            className="w-full"
          />
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          <Label htmlFor={`${id}-target`}>目标</Label>
          <NodePicker
            id={`${id}-target`}
            data-testid="preview-target"
            nodes={active}
            value={flow.targetNodeId}
            noneLabel="外部目标（域名 / IP）"
            // External hints never carry over into a mesh-node query.
            onValueChange={(targetNodeId) =>
              onFlowChange(
                targetNodeId === ""
                  ? { ...flow, targetNodeId }
                  : { ...flow, targetNodeId, domain: "", ip: "", country: "" }
              )
            }
            placeholder="选择目标"
            disabled={disabled}
            className="w-full"
          />
        </div>
      </div>
      {external ? (
        <div className="grid gap-3 sm:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)_4.5rem]">
          <div className="flex min-w-0 flex-col gap-1.5">
            <Label htmlFor={`${id}-domain`}>域名</Label>
            <Input
              id={`${id}-domain`}
              value={flow.domain}
              onChange={(event) => onFlowChange({ ...flow, domain: event.target.value })}
              placeholder="www.example.com"
              autoComplete="off"
              spellCheck={false}
              disabled={disabled}
              className="font-mono"
            />
          </div>
          <div className="flex min-w-0 flex-col gap-1.5">
            <Label htmlFor={`${id}-ip`}>IP</Label>
            <Input
              id={`${id}-ip`}
              value={flow.ip}
              onChange={(event) => onFlowChange({ ...flow, ip: event.target.value })}
              placeholder="203.0.113.10"
              autoComplete="off"
              spellCheck={false}
              disabled={disabled}
              className="font-mono"
            />
          </div>
          <div className="flex min-w-0 flex-col gap-1.5">
            <Label htmlFor={`${id}-country`} title="手动提示，controller 不做 GeoIP">
              国家
            </Label>
            <Input
              id={`${id}-country`}
              value={flow.country}
              onChange={(event) => onFlowChange({ ...flow, country: event.target.value })}
              placeholder="JP"
              maxLength={2}
              autoComplete="off"
              spellCheck={false}
              disabled={disabled}
              className="font-mono uppercase"
            />
          </div>
          <p className="text-[0.6875rem] text-muted-foreground sm:col-span-3">
            至少填写域名或 IP。Mesh 地址段内的 IP 按对应节点处理；国家只是手动提示。
          </p>
        </div>
      ) : null}
    </div>
  )
}

export function PreviewOutcome({
  result,
  nodes,
  flow,
  stale,
  pending,
  staleMessage,
  revisionNote,
  onRuleClick,
  emptyMessage,
}: {
  result: PreviewResult | null
  nodes: NodeSummary[]
  flow: FlowDraft
  stale: boolean
  pending: boolean
  staleMessage: string
  revisionNote?: (response: PreviewPolicyResponse) => string | null
  onRuleClick?: (ruleId: string) => void
  emptyMessage: string
}) {
  return (
    <section
      data-testid="route-preview"
      aria-label="预览结果"
      aria-live="polite"
      aria-busy={pending || undefined}
      className="flex flex-col gap-3"
    >
      {pending ? (
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <Spinner className="size-3.5" role="presentation" aria-hidden="true" />
          正在编译…
        </p>
      ) : null}
      {result === null ? (
        pending ? null : <p className="text-xs text-muted-foreground">{emptyMessage}</p>
      ) : result.status === "error" ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
          <span className="break-all">{result.message}</span>
        </p>
      ) : (
        <div className={stale ? "flex flex-col gap-3 opacity-60" : "flex flex-col gap-3"}>
          {stale ? (
            <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <HistoryIcon className="size-3.5" />
              {staleMessage}
            </p>
          ) : null}
          <Decision
            response={result.response}
            nodes={nodes}
            flow={flow}
            onRuleClick={onRuleClick}
          />
          {revisionNote?.(result.response) ? (
            <p role="status" className="text-xs text-warning">
              {revisionNote(result.response)}
            </p>
          ) : null}
        </div>
      )}
    </section>
  )
}

function Decision({
  response,
  nodes,
  flow,
  onRuleClick,
}: {
  response: PreviewPolicyResponse
  nodes: NodeSummary[]
  flow: FlowDraft
  onRuleClick?: (ruleId: string) => void
}) {
  const { decision } = response
  const badge = decisionBadge[decision.action]
  const steps = planSteps(decision, externalLabel(flow))
  const ruleId = decision.rule_id

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Badge variant={badge.variant}>{badge.label}</Badge>
        <span className="text-xs">
          {ruleId !== null ? (
            <>
              命中规则{" "}
              {onRuleClick ? (
                <button
                  type="button"
                  onClick={() => onRuleClick(ruleId)}
                  className="font-mono font-medium underline underline-offset-2 hover:text-brand"
                >
                  {ruleId}
                </button>
              ) : (
                <code className="font-mono font-medium">{ruleId}</code>
              )}
            </>
          ) : decision.action === "invalid" ? (
            "策略或预览输入未通过编译"
          ) : (
            "没有规则命中，使用默认动作"
          )}
        </span>
      </div>
      {steps.length > 0 ? <PathTrace steps={steps} nodes={nodes} /> : null}
      <p className="text-xs text-muted-foreground">{decision.reason}</p>
      {!response.valid ? (
        <DiagnosticList label="编译诊断" diagnostics={response.diagnostics} showRule />
      ) : null}
    </div>
  )
}

export function PreviewButton({
  onClick,
  disabled,
  pending,
  blockedReason,
  label = "编译预览",
}: {
  onClick: () => void
  disabled: boolean
  pending: boolean
  blockedReason: string | null
  label?: string
}) {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button type="button" onClick={onClick} disabled={disabled || blockedReason !== null}>
        {pending ? (
          <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" />
        ) : (
          <PlayIcon data-icon="inline-start" />
        )}
        {label}
      </Button>
      {blockedReason ? (
        <span className="text-xs text-muted-foreground">{blockedReason}</span>
      ) : null}
    </div>
  )
}
