import type { NodeSummary, RouteDecision } from "@meshway/control-client"
import { GlobeIcon } from "lucide-react"

import { nodeRefLabel, resolveNode } from "@/lib/policy-model"
import { cn } from "@/lib/utils"

export type PathStep =
  | { kind: "node"; id: string; role: "source" | "via" | "exit" | "target" }
  | { kind: "external"; label: string }

const roleLabels = {
  source: "来源",
  via: "经由",
  exit: "出口",
  target: "目标",
} as const

/**
 * Turns a controller-compiled decision into display steps. It never infers
 * hops: the node sequence is exactly `decision.path`. A trailing external
 * step is shown when the plan leaves the mesh (explicit exit, or a local
 * egress with a single-node path for an external target).
 */
export function planSteps(
  decision: RouteDecision,
  externalLabel: string | null
): PathStep[] {
  const { path, exit_node_id: exit } = decision
  const external = exit !== null || (externalLabel !== null && path.length <= 1)
  const steps: PathStep[] = path.map((id, index) => {
    const last = index === path.length - 1
    const role =
      index === 0
        ? "source"
        : last && exit === id
          ? "exit"
          : last && !external
            ? "target"
            : "via"
    return { kind: "node", id, role }
  })
  if (external && steps.length > 0) {
    steps.push({ kind: "external", label: externalLabel ?? "外部目标" })
  }
  return steps
}

/**
 * Ordered logical plan. Links between steps carry no measurement: latency is
 * shown as not measured until the data plane reports it.
 */
export function PathTrace({
  steps,
  nodes,
  orientation = "horizontal",
  className,
}: {
  steps: PathStep[]
  nodes: NodeSummary[]
  orientation?: "horizontal" | "vertical"
  className?: string
}) {
  const vertical = orientation === "vertical"
  return (
    <ol
      aria-label="规划路径"
      className={cn(
        "flex",
        vertical ? "flex-col" : "flex-wrap items-center gap-y-2",
        className
      )}
    >
      {steps.map((step, index) => (
        <li
          key={`${index}-${step.kind === "node" ? step.id : "external"}`}
          className={cn("flex", vertical ? "flex-col" : "items-center")}
        >
          {index > 0 ? <Link vertical={vertical} /> : null}
          <Step step={step} nodes={nodes} />
        </li>
      ))}
    </ol>
  )
}

function Link({ vertical }: { vertical: boolean }) {
  return vertical ? (
    <span className="ml-3.5 flex h-7 items-center gap-2 border-l border-dashed border-brand/60 pl-4">
      <span className="font-mono text-[0.625rem] text-muted-foreground">RTT — 未测量</span>
    </span>
  ) : (
    <span aria-hidden="true" className="flex items-center px-1">
      <span className="h-px w-5 bg-brand/60" />
      <span className="size-0 border-y-[3px] border-l-[5px] border-y-transparent border-l-brand/60" />
    </span>
  )
}

function Step({ step, nodes }: { step: PathStep; nodes: NodeSummary[] }) {
  if (step.kind === "external") {
    return (
      <span className="inline-flex items-center gap-2 rounded-lg border border-dashed px-2.5 py-1.5">
        <GlobeIcon className="size-3.5 text-muted-foreground" />
        <span className="flex flex-col leading-tight">
          <span className="text-[0.625rem] text-muted-foreground">外部</span>
          <span className="max-w-48 truncate font-mono text-xs">{step.label}</span>
        </span>
      </span>
    )
  }
  const ref = resolveNode(nodes, step.id)
  return (
    <span
      className={cn(
        "inline-flex items-center gap-2 rounded-lg border bg-background px-2.5 py-1.5",
        ref.kind !== "active" && "border-destructive/50 text-destructive",
        (step.role === "source" || step.role === "target" || step.role === "exit") &&
          ref.kind === "active" &&
          "border-brand/50"
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          "flex size-5 items-center justify-center rounded-full text-[0.625rem] font-semibold",
          step.role === "via" ? "bg-muted text-muted-foreground" : "bg-brand text-brand-foreground"
        )}
      >
        {roleLabels[step.role].slice(0, 1)}
      </span>
      <span className="flex flex-col leading-tight">
        <span className="text-[0.625rem] text-muted-foreground">{roleLabels[step.role]}</span>
        <span className="max-w-40 truncate text-xs font-medium">{nodeRefLabel(ref)}</span>
      </span>
    </span>
  )
}
