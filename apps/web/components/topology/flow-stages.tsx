import type { NodeSummary } from "@meshway/control-client"
import { ArrowRightIcon } from "lucide-react"
import Link from "next/link"

import { Badge } from "@/components/ui/badge"
import { stageLabels, type FlowStage, type StageStatus } from "@/lib/flow-stages"
import { channelStatusLabels } from "@/lib/topology-model"
import { cn } from "@/lib/utils"

const statusLabels: Record<StageStatus, string> = {
  planned: "已规划",
  rejected: "拒绝",
  observed: "已观测",
  missing: "未观测",
  unknown: "未知",
}

const statusClass: Record<StageStatus, string> = {
  planned: "border-brand/50 text-brand",
  rejected: "border-destructive/50 text-destructive",
  observed: "border-success/50 text-success",
  missing: "border-warning/50 text-warning",
  unknown: "text-muted-foreground",
}

function StatusBadge({ status }: { status: StageStatus }) {
  return (
    <Badge variant="outline" className={cn("shrink-0", statusClass[status])}>
      {statusLabels[status]}
    </Badge>
  )
}

/**
 * The evidence for one planned flow, one stage per source of truth. Planned
 * is not observed, and an observed neighbor is not a delivered packet: each
 * stage says which it is, and links to where it is configured.
 */
export function FlowStages({ stages, nodes }: { stages: FlowStage[]; nodes: NodeSummary[] }) {
  const name = (id: string) => nodes.find((node) => node.id === id)?.name ?? `${id.slice(0, 12)}…`
  return (
    <ol aria-label="流量证据" data-testid="flow-stages" className="flex flex-col gap-3">
      {stages.map((stage, index) => (
        <li key={stage.key} className="flex gap-3">
          <span
            aria-hidden="true"
            className="flex size-5 shrink-0 items-center justify-center rounded-full border font-mono text-[0.625rem] text-muted-foreground"
          >
            {index + 1}
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex items-center gap-2">
              <span className="text-xs font-medium">{stageLabels[stage.key]}</span>
              <StatusBadge status={stage.status} />
            </div>
            <StageDetail stage={stage} name={name} />
          </div>
        </li>
      ))}
    </ol>
  )
}

function StageDetail({ stage, name }: { stage: FlowStage; name: (id: string) => string }) {
  const muted = "text-[0.6875rem] text-muted-foreground"
  switch (stage.key) {
    case "route":
      return (
        <div className="flex flex-col gap-1">
          <p className={muted}>
            {stage.ruleId ? (
              <>
                命中规则{" "}
                <Link
                  href={`/routing?rule=${encodeURIComponent(stage.ruleId)}`}
                  className="font-mono text-foreground underline underline-offset-2"
                >
                  {stage.ruleId}
                </Link>
              </>
            ) : (
              "没有规则命中，使用默认动作"
            )}
            {" · "}
            {stage.reason}
          </p>
        </div>
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
                <Link href="/connections" className="underline underline-offset-2">
                  配置连接
                </Link>
              </>
            ) : null}
          </p>
        </div>
      )
    case "access":
      return (
        <p className={muted}>
          {stage.reason}{" "}
          <Link href="/access" className="underline underline-offset-2">
            查看访问授权
          </Link>
        </p>
      )
    case "dataplane":
      return <p className={muted}>{stage.reason}</p>
  }
}
