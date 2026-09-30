"use client"

import type { NodeSummary, PreviewPolicyResponse } from "@meshway/control-client"
import { ArrowRightIcon, CircleAlertIcon, PlayIcon } from "lucide-react"
import { useId } from "react"

import { DiagnosticList } from "@/components/routing/diagnostics"
import { NodeSelect } from "@/components/routing/node-select"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { PreviewResult } from "@/lib/action-state"
import { nodeRefLabel, resolveNode, type FlowDraft } from "@/lib/policy-model"

const decisionBadge = {
  route: { label: "路由", variant: "default" },
  reject: { label: "拒绝", variant: "destructive" },
  invalid: { label: "策略无效", variant: "destructive" },
} as const

function NodeChip({ nodes, id }: { nodes: NodeSummary[]; id: string }) {
  const ref = resolveNode(nodes, id)
  return (
    <span className="inline-flex flex-col rounded-md border px-2 py-1">
      <span className="text-xs font-medium">{nodeRefLabel(ref)}</span>
      <code className="font-mono text-[0.625rem] text-muted-foreground">{id}</code>
    </span>
  )
}

function Outcome({
  response,
  nodes,
  baseRevision,
}: {
  response: PreviewPolicyResponse
  nodes: NodeSummary[]
  baseRevision: number
}) {
  const { decision } = response
  const badge = decisionBadge[decision.action]
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Badge variant={badge.variant}>{badge.label}</Badge>
        <span>
          {decision.rule_id !== null ? (
            <>
              命中规则 <code className="font-mono text-xs">{decision.rule_id}</code>
            </>
          ) : decision.action === "invalid" ? (
            "策略未通过编译"
          ) : (
            "没有规则命中，使用默认动作"
          )}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">{decision.reason}</p>

      {decision.path.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium">规划路径</span>
          <ol aria-label="规划路径" className="flex flex-wrap items-center gap-1.5">
            {decision.path.map((id, index) => (
              <li key={`${index}-${id}`} className="flex items-center gap-1.5">
                {index > 0 ? (
                  <ArrowRightIcon className="size-3.5 text-muted-foreground" aria-hidden />
                ) : null}
                <NodeChip nodes={nodes} id={id} />
              </li>
            ))}
          </ol>
        </div>
      ) : null}
      {decision.exit_node_id !== null ? (
        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium">出口节点</span>
          <div>
            <NodeChip nodes={nodes} id={decision.exit_node_id} />
          </div>
        </div>
      ) : null}
      <p className="text-xs text-muted-foreground">
        规划路径是 controller 按策略和当前活动节点清单编译出的逻辑路径，不是实际
        traceroute，也不检查链路可达性或访问授权；节点之间的实际转发尚未实现。
      </p>

      {!response.valid ? (
        <DiagnosticList
          label="编译诊断"
          diagnostics={response.diagnostics}
          showRule
        />
      ) : null}
      {response.revision !== baseRevision ? (
        <p role="status" className="text-xs text-destructive">
          编译时 controller 的配置版本为 {response.revision}，而草稿基于版本 {baseRevision}
          ：节点清单或策略已被修改。编辑基础不会自动前移，此时保存会被拒绝。
        </p>
      ) : null}
    </div>
  )
}

export function RoutePreview({
  nodes,
  flow,
  onFlowChange,
  result,
  stale,
  pending,
  disabled,
  blockedReason,
  onPreview,
  baseRevision,
}: {
  nodes: NodeSummary[]
  flow: FlowDraft
  onFlowChange: (flow: FlowDraft) => void
  result: PreviewResult | null
  stale: boolean
  pending: boolean
  disabled: boolean
  blockedReason: string | null
  onPreview: () => void
  baseRevision: number
}) {
  const id = useId()
  const external = flow.targetNodeId === ""

  return (
    <Card className="[--card-spacing:--spacing(5)]">
      <CardHeader>
        <CardTitle className="text-base">编译预览</CardTitle>
        <CardDescription className="text-sm">
          用当前草稿（未保存）和一条假设的流量让 controller 编译并给出决策，不会保存任何内容。
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="flex flex-col gap-2">
            <Label htmlFor={`${id}-source`}>预览源节点</Label>
            <NodeSelect
              id={`${id}-source`}
              data-testid="preview-source"
              nodes={nodes.filter((node) => node.revoked_at === null)}
              value={flow.sourceNodeId}
              onValueChange={(sourceNodeId) => onFlowChange({ ...flow, sourceNodeId })}
              placeholder="选择源节点"
              className="w-full"
            />
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor={`${id}-target`}>预览目标</Label>
            <NodeSelect
              id={`${id}-target`}
              data-testid="preview-target"
              nodes={nodes.filter((node) => node.revoked_at === null)}
              value={flow.targetNodeId}
              // External hints never carry over into a mesh-node query.
              onValueChange={(targetNodeId) =>
                onFlowChange(
                  targetNodeId === ""
                    ? { ...flow, targetNodeId }
                    : { ...flow, targetNodeId, domain: "", ip: "", country: "" }
                )
              }
              placeholder="Mesh 外部目标（域名 / IP）"
              className="w-full"
            />
          </div>
        </div>
        {external ? (
          <div className="grid gap-3 sm:grid-cols-3">
            <div className="flex flex-col gap-2">
              <Label htmlFor={`${id}-domain`}>预览目标域名</Label>
              <Input
                id={`${id}-domain`}
                value={flow.domain}
                onChange={(event) => onFlowChange({ ...flow, domain: event.target.value })}
                placeholder="www.example.com"
                autoComplete="off"
                spellCheck={false}
                className="font-mono"
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor={`${id}-ip`}>预览目标 IP</Label>
              <Input
                id={`${id}-ip`}
                value={flow.ip}
                onChange={(event) => onFlowChange({ ...flow, ip: event.target.value })}
                placeholder="203.0.113.10"
                autoComplete="off"
                spellCheck={false}
                className="font-mono"
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor={`${id}-country`}>预览目标国家（手动提示）</Label>
              <Input
                id={`${id}-country`}
                value={flow.country}
                onChange={(event) => onFlowChange({ ...flow, country: event.target.value })}
                placeholder="可选，如 JP"
                maxLength={2}
                autoComplete="off"
                spellCheck={false}
                className="font-mono"
              />
            </div>
            <p className="text-xs text-muted-foreground sm:col-span-3">
              未填写的域名或 IP 视为未知，不会匹配对应条件。填写 Mesh 地址段内的 IP
              时按 Mesh 节点处理；国家只是预览提示，controller 不做 GeoIP。
            </p>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">
            Mesh 目标的国家来自管理员为该节点标注的元数据。
          </p>
        )}

        <div className="flex flex-wrap items-center gap-3">
          <Button type="button" size="lg" disabled={disabled || blockedReason !== null} onClick={onPreview}>
            <PlayIcon data-icon="inline-start" />
            {pending ? "正在编译" : "编译预览"}
          </Button>
          {blockedReason ? (
            <span className="text-xs text-muted-foreground">{blockedReason}</span>
          ) : null}
        </div>

        <section
          data-testid="route-preview"
          aria-label="预览结果"
          aria-live="polite"
          aria-busy={pending || undefined}
          className="flex flex-col gap-3 rounded-md border px-4 py-3"
        >
          {result === null ? (
            <p className="text-xs text-muted-foreground">尚未编译预览。</p>
          ) : result.status === "error" ? (
            <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
              <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
              <span className="break-all">{result.message}</span>
            </p>
          ) : (
            <>
              {stale ? (
                <p className="text-xs font-medium text-muted-foreground">
                  草稿或预览流量已修改，以下是上一次编译的结果。
                </p>
              ) : null}
              <Outcome response={result.response} nodes={nodes} baseRevision={baseRevision} />
            </>
          )}
        </section>
      </CardContent>
    </Card>
  )
}
