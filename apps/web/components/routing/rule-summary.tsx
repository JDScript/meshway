import type { NodeSummary } from "@meshway/control-client"
import { BanIcon, GlobeIcon } from "lucide-react"
import type { ReactNode } from "react"

import {
  nodeRefLabel,
  parseCountryList,
  resolveNode,
  type DestinationKind,
  type DestinationScope,
  type RuleDraft,
} from "@/lib/policy-model"
import { cn } from "@/lib/utils"

export const scopeLabels: Record<DestinationScope, string> = {
  any: "任意目标",
  mesh: "Mesh 节点",
  internet: "互联网",
}

export const kindLabels: Record<DestinationKind, string> = {
  node: "节点",
  domain_suffix: "域名后缀",
  cidr: "CIDR",
  country: "国家",
}

function Chip({
  children,
  tone = "default",
  className,
}: {
  children: ReactNode
  tone?: "default" | "muted" | "danger" | "broad"
  className?: string
}) {
  return (
    <span
      className={cn(
        "inline-flex h-5 max-w-full min-w-0 items-center gap-1 truncate rounded-md border px-1.5 text-[0.6875rem]",
        tone === "default" && "bg-background",
        tone === "muted" && "border-transparent bg-muted text-muted-foreground",
        tone === "danger" && "border-destructive/40 text-destructive",
        tone === "broad" && "border-dashed text-muted-foreground",
        className
      )}
    >
      {children}
    </span>
  )
}

function NodeChip({ nodes, id }: { nodes: NodeSummary[]; id: string }) {
  if (!id) return <Chip tone="danger">未选择</Chip>
  const ref = resolveNode(nodes, id)
  return <Chip tone={ref.kind === "active" ? "default" : "danger"}>{nodeRefLabel(ref)}</Chip>
}

const MAX_CHIPS = 3

function Overflow({ count }: { count: number }) {
  return count > 0 ? <Chip tone="muted">+{count}</Chip> : null
}

/**
 * Labelled source / target / action summary of a draft rule. Groups are
 * separated neutrally, not by arrows: the only ordered sequence shown is the
 * via list itself, and the true planned path belongs to the preview.
 */
export function RuleSummary({ rule, nodes }: { rule: RuleDraft; nodes: NodeSummary[] }) {
  const countries = parseCountryList(rule.sourceCountries)
  const sources = [
    ...rule.sourceNodeIds.map((id) => <NodeChip key={`n-${id}`} nodes={nodes} id={id} />),
    ...countries.map((country) => (
      <Chip key={`c-${country}`} className="font-mono">
        {country}
      </Chip>
    )),
  ]
  const targets = rule.destinations.map((selector) => (
    <Chip key={selector.key} className={selector.kind === "node" ? undefined : "font-mono"}>
      <span className="text-muted-foreground">{kindLabels[selector.kind]}</span>
      {selector.kind === "node" ? (
        nodeRefLabel(resolveNode(nodes, selector.value))
      ) : (
        <span className="truncate">{selector.value || "—"}</span>
      )}
    </Chip>
  ))

  return (
    <span className="flex flex-col gap-1.5 text-xs sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-3">
      <Segment label="来源">
        {sources.length === 0 ? <Chip tone="broad">任意来源</Chip> : sources.slice(0, MAX_CHIPS)}
        <Overflow count={sources.length - MAX_CHIPS} />
      </Segment>
      <Divider />
      <Segment label="目标">
        <Chip tone="muted">{scopeLabels[rule.scope]}</Chip>
        {targets.length === 0 ? <Chip tone="broad">范围内全部</Chip> : targets.slice(0, MAX_CHIPS)}
        <Overflow count={targets.length - MAX_CHIPS} />
      </Segment>
      <Divider />
      <Segment label="动作">
        <ActionSummary rule={rule} nodes={nodes} />
      </Segment>
    </span>
  )
}

function Divider() {
  return <span aria-hidden="true" className="hidden h-3.5 w-px shrink-0 bg-border sm:block" />
}

function Segment({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <span className="w-8 shrink-0 text-[0.6875rem] text-muted-foreground sm:w-auto">{label}</span>
      <span className="flex min-w-0 flex-wrap items-center gap-1">{children}</span>
    </span>
  )
}

export function ActionSummary({ rule, nodes }: { rule: RuleDraft; nodes: NodeSummary[] }) {
  if (rule.action === "reject") {
    return (
      <Chip tone="danger">
        <BanIcon className="size-3" />
        拒绝
      </Chip>
    )
  }
  if (rule.via.length === 0 && rule.exitNodeId === "") {
    return <Chip>{rule.scope === "mesh" ? "路由 · 直达目标" : "路由 · 本地直连"}</Chip>
  }
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-1">
      <Chip tone="muted">路由</Chip>
      {rule.via.length > 0 ? (
        <span className="flex min-w-0 flex-wrap items-center gap-1">
          <span className="text-[0.6875rem] text-muted-foreground">经由</span>
          {rule.via.map((id, index) => (
            <span key={`${index}-${id}`} className="flex items-center gap-1">
              {index > 0 ? (
                <span aria-label="然后" className="text-[0.625rem] text-muted-foreground">
                  ›
                </span>
              ) : null}
              <NodeChip nodes={nodes} id={id} />
            </span>
          ))}
        </span>
      ) : null}
      {rule.exitNodeId !== "" ? (
        <span className="flex items-center gap-1">
          <span className="text-[0.6875rem] text-muted-foreground">出口</span>
          <Chip>
            <GlobeIcon className="size-3 text-brand" />
            {nodeRefLabel(resolveNode(nodes, rule.exitNodeId))}
          </Chip>
        </span>
      ) : null}
    </span>
  )
}
