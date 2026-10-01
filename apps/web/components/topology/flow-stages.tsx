"use client"

import type { NodeSummary } from "@meshway/control-client"
import { useRoutes } from "@/components/shell/network-context"
import { ArrowLeftRightIcon, ArrowRightIcon } from "lucide-react"
import Link from "next/link"

import { Badge } from "@/components/ui/badge"
import { stageLabels, type FlowStage, type StageStatus } from "@/lib/flow-stages"
import { channelStatusLabels } from "@/lib/topology-model"
import { cn } from "@/lib/utils"

const statusLabels: Record<StageStatus, string> = {
  allowed: "允许",
  rejected: "拒绝",
  invalid: "策略无效",
  planned: "已规划",
  observed: "已观测",
  missing: "未观测",
  eligible: "满足",
  ineligible: "不满足",
  unknown: "未知",
}

const statusClass: Record<StageStatus, string> = {
  allowed: "border-success/50 text-success",
  rejected: "border-destructive/50 text-destructive",
  invalid: "border-destructive/50 text-destructive",
  planned: "border-brand/50 text-brand",
  observed: "border-success/50 text-success",
  missing: "border-warning/50 text-warning",
  eligible: "border-success/50 text-success",
  ineligible: "border-warning/50 text-warning",
  unknown: "text-muted-foreground",
}

/**
 * The evidence for one flow, one stage per source of truth. Allowed is not
 * routed, planned is not observed, and an eligible runtime path is not a
 * delivered packet: each stage says which it is and links to where it is
 * configured. `skip` hides stages a page has no data for.
 */
export function FlowStages({
  stages,
  nodes,
  skip = [],
}: {
  stages: FlowStage[]
  nodes: NodeSummary[]
  skip?: FlowStage["key"][]
}) {
  const name = (id: string) => nodes.find((node) => node.id === id)?.name ?? `${id.slice(0, 12)}…`
  const shown = stages.filter((stage) => !skip.includes(stage.key))
  return (
    <ol aria-label="流量证据" data-testid="flow-stages" className="flex flex-col gap-3">
      {shown.map((stage, index) => (
        <li key={stage.key} data-testid={`flow-stage-${stage.key}`} data-status={stage.status} className="flex gap-3">
          <span
            aria-hidden="true"
            className="flex size-5 shrink-0 items-center justify-center rounded-full border font-mono text-[0.625rem] text-muted-foreground"
          >
            {index + 1}
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex items-center gap-2">
              <span className="text-xs font-medium">{stageLabels[stage.key]}</span>
              <Badge variant="outline" className={cn("shrink-0", statusClass[stage.status])}>
                {statusLabels[stage.status]}
              </Badge>
            </div>
            <StageDetail stage={stage} name={name} />
          </div>
        </li>
      ))}
    </ol>
  )
}

const muted = "text-[0.6875rem] text-muted-foreground break-words"

function StageDetail({ stage, name }: { stage: FlowStage; name: (id: string) => string }) {
  const routes = useRoutes()
  switch (stage.key) {
    case "access":
      return (
        <p className={muted}>
          {stage.ruleId ? (
            <>
              命中访问规则{" "}
              <Link
                href={routes.access({ rule: stage.ruleId })}
                className="font-mono text-foreground underline underline-offset-2"
              >
                {stage.ruleId}
              </Link>
              {stage.direction === "reverse" ? (
                <span className="ml-1 inline-flex items-center gap-0.5">
                  （双向规则的反向
                  <ArrowLeftRightIcon aria-hidden="true" className="size-3" />）
                </span>
              ) : stage.direction === "forward" ? (
                <span className="ml-1 inline-flex items-center gap-0.5">
                  （正向
                  <ArrowRightIcon aria-hidden="true" className="size-3" />）
                </span>
              ) : null}
            </>
          ) : (
            <>
              没有访问规则命中，使用默认动作 ·{" "}
              <Link href={routes.access()} className="underline underline-offset-2">
                访问授权
              </Link>
            </>
          )}
          {" · "}
          {stage.reason}
        </p>
      )
    case "route":
      return (
        <p className={muted}>
          {stage.ruleId ? (
            <>
              命中路由规则{" "}
              <Link
                href={routes.routing({ rule: stage.ruleId })}
                className="font-mono text-foreground underline underline-offset-2"
              >
                {stage.ruleId}
              </Link>
            </>
          ) : (
            "没有路由规则命中，使用默认动作"
          )}
          {stage.path.length > 0 ? ` · ${stage.path.map(name).join(" → ")}` : ""}
          {" · "}
          {stage.reason}
        </p>
      )
    case "neighbors":
      return (
        <div className="flex flex-col gap-1">
          {stage.hops.length > 0 ? (
            <ul aria-label="规划路径的每一跳" className="flex flex-col gap-1">
              {stage.hops.map((hop) => (
                <li key={`${hop.from}-${hop.to}`} className="flex flex-wrap items-center gap-1.5 text-xs">
                  <span className="font-medium">{name(hop.from)}</span>
                  <ArrowRightIcon aria-label="到" className="size-3 text-muted-foreground" />
                  <span className="font-medium">{name(hop.to)}</span>
                  <span className={cn("ml-auto", muted)}>
                    {hop.adjacency === null ? "未配置连接" : channelStatusLabels[hop.adjacency]}
                  </span>
                </li>
              ))}
            </ul>
          ) : null}
          <p className={muted}>
            {stage.reason}
            {stage.hops.some((hop) => hop.adjacency === null) ? (
              <>
                {" "}
                <Link href={routes.connections} className="underline underline-offset-2">
                  配置连接
                </Link>
              </>
            ) : null}
          </p>
        </div>
      )
    case "execution":
      return (
        <p className={muted}>
          {stage.reason}。这是 controller 对已保存配置的判断，不代表报文已送达；实际连通请在节点上用 ping / curl 测试。
        </p>
      )
  }
}
