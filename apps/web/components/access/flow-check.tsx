"use client"

import type { NodeSummary } from "@meshway/control-client"
import { ArrowUpRightIcon, CircleAlertIcon, HistoryIcon } from "lucide-react"
import Link from "next/link"
import { useRef, useState, useTransition } from "react"

import { Panel } from "@/components/common/panel"
import { FlowForm, PreviewButton } from "@/components/routing/route-preview"
import { FlowStages } from "@/components/topology/flow-stages"
import { Badge } from "@/components/ui/badge"
import { previewFlowAction } from "@/lib/access-actions"
import type { FlowPreviewResult } from "@/lib/action-state"
import { flowStages } from "@/lib/flow-stages"
import { emptyFlow, toFlow, validateFlow, type FlowDraft } from "@/lib/policy-model"

/**
 * Asks the controller how one flow is decided under the *saved* access and
 * routing policies. The access decision, the matched rule and its direction
 * all come from the controller; this component evaluates nothing itself.
 */
export function FlowCheck({
  nodes,
  baseRevision,
  dirty,
}: {
  nodes: NodeSummary[]
  baseRevision: number
  /** The page holds unsaved changes, which the check does not see. */
  dirty: boolean
}) {
  const [flow, setFlow] = useState<FlowDraft>(emptyFlow)
  const [result, setResult] = useState<{ result: FlowPreviewResult; key: string } | null>(null)
  const [pending, start] = useTransition()
  const seq = useRef(0)
  const key = JSON.stringify(toFlow(flow))
  const flowError = validateFlow(flow)

  const run = () => {
    const current = ++seq.current
    const query = toFlow(flow)
    const submitted = key
    start(async () => {
      const next = await previewFlowAction(query)
      if (current === seq.current) setResult({ result: next, key: submitted })
    })
  }

  const response = result?.result.status === "previewed" ? result.result.response : null
  const stale = result !== null && result.key !== key
  const topologyHref = flow.sourceNodeId ? `/topology?node=${encodeURIComponent(flow.sourceNodeId)}` : "/topology"

  return (
    <Panel
      data-testid="access-flow-check"
      title={
        <span className="flex items-center gap-2">
          检查一条流量
          <Badge variant="outline">已保存配置</Badge>
        </span>
      }
      description="Controller 按已保存的访问规则与路由策略判断：是否允许、命中哪条规则、走哪条路径。是计划，不是 trace。"
      bodyClassName="flex flex-col gap-4 p-4"
    >
      <FlowForm nodes={nodes} flow={flow} onFlowChange={setFlow} disabled={pending} />
      <PreviewButton onClick={run} disabled={pending} pending={pending} blockedReason={flowError} label="检查" />
      {dirty ? (
        <p className="text-xs text-warning">草稿中未保存的修改不参与检查；保存后再检查。</p>
      ) : null}
      {result?.result.status === "error" ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
          <span className="break-all">{result.result.message}</span>
        </p>
      ) : null}
      {response ? (
        <div className={stale ? "flex flex-col gap-3 opacity-60" : "flex flex-col gap-3"} aria-live="polite">
          {stale ? (
            <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <HistoryIcon className="size-3.5" />
              流量已修改，以下是上一次的结果。
            </p>
          ) : null}
          {response.revision !== baseRevision ? (
            <p className="text-xs text-muted-foreground">检查基于 controller 当前的 r{response.revision}（编辑基础为 r{baseRevision}）。</p>
          ) : null}
          <FlowStages stages={flowStages(response, null, false)} nodes={nodes} skip={["neighbors"]} />
          <Link
            href={topologyHref}
            className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            在网络拓扑中查看逐跳邻接
            <ArrowUpRightIcon className="size-3" />
          </Link>
        </div>
      ) : null}
    </Panel>
  )
}
