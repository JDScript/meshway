import type { NodeSummary } from "@meshway/control-client"
import { BanIcon, ChevronRightIcon, GlobeIcon } from "lucide-react"
import { Fragment, type ReactNode } from "react"

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

const MAX_CHIPS = 4

function Joiner({ children }: { children: ReactNode }) {
  return <span className="text-[0.625rem] text-muted-foreground">{children}</span>
}

/** Chips joined by "或", truncated with a count. */
function AnyOf({ items }: { items: ReactNode[] }) {
  const shown = items.slice(0, MAX_CHIPS)
  return (
    <>
      {shown.map((item, index) => (
        <Fragment key={index}>
          {index > 0 ? <Joiner>或</Joiner> : null}
          {item}
        </Fragment>
      ))}
      {items.length > MAX_CHIPS ? <Chip tone="muted">另 {items.length - MAX_CHIPS} 项</Chip> : null}
    </>
  )
}

function sourceParts(rule: RuleDraft, nodes: NodeSummary[]) {
  const countries = parseCountryList(rule.sourceCountries)
  return {
    nodes: rule.sourceNodeIds.map((id) => <NodeChip key={`n-${id}`} nodes={nodes} id={id} />),
    countries: countries.map((country) => (
      <Chip key={`c-${country}`} className="font-mono">
        {country}
      </Chip>
    )),
  }
}

/**
 * Readable source / target / action of a draft rule, with the matching
 * logic spelled out: alternatives inside a group are OR, node and country
 * sources must both hold (AND), and the target scope always applies. The
 * only ordered sequence shown is the via list; the planned path for a real
 * flow belongs to the controller preview.
 */
export function RuleSummary({ rule, nodes }: { rule: RuleDraft; nodes: NodeSummary[] }) {
  const source = sourceParts(rule, nodes)
  const targets = rule.destinations.map((selector) => (
    <Chip key={selector.key} className={selector.kind === "node" ? undefined : "font-mono"}>
      <span className="font-sans text-muted-foreground">{kindLabels[selector.kind]}</span>
      {selector.kind === "node" ? (
        nodeRefLabel(resolveNode(nodes, selector.value))
      ) : (
        <span className="truncate">{selector.value || "—"}</span>
      )}
    </Chip>
  ))

  return (
    <span className="grid grid-cols-[2.25rem_minmax(0,1fr)] items-start gap-x-2 gap-y-1.5 text-xs">
      <Label>来源</Label>
      <Line>
        {source.nodes.length === 0 && source.countries.length === 0 ? (
          <Chip tone="broad">任意来源</Chip>
        ) : (
          <>
            {source.nodes.length > 0 ? <AnyOf items={source.nodes} /> : null}
            {source.nodes.length > 0 && source.countries.length > 0 ? <Joiner>且国家为</Joiner> : null}
            {source.countries.length > 0 ? (
              <>
                {source.nodes.length === 0 ? <Joiner>国家为</Joiner> : null}
                <AnyOf items={source.countries} />
              </>
            ) : null}
          </>
        )}
      </Line>
      <Label>目标</Label>
      <Line>
        <Chip tone="muted">{scopeLabels[rule.scope]}</Chip>
        {targets.length === 0 ? (
          <Chip tone="broad">范围内全部</Chip>
        ) : (
          <>
            <Joiner>中匹配</Joiner>
            <AnyOf items={targets} />
          </>
        )}
      </Line>
      <Label>动作</Label>
      <Line>
        <ActionSummary rule={rule} nodes={nodes} />
      </Line>
    </span>
  )
}

function Label({ children }: { children: ReactNode }) {
  return <span className="pt-0.5 text-[0.6875rem] text-muted-foreground">{children}</span>
}

function Line({ children }: { children: ReactNode }) {
  return <span className="flex min-w-0 flex-wrap items-center gap-1">{children}</span>
}

/** Route path as an ordered chain: source › via… › exit → destination. */
export function ActionSummary({ rule, nodes }: { rule: RuleDraft; nodes: NodeSummary[] }) {
  if (rule.action === "reject") {
    return (
      <Chip tone="danger">
        <BanIcon className="size-3" />
        拒绝
      </Chip>
    )
  }
  const step = <ChevronRightIcon aria-hidden="true" className="size-3 shrink-0 text-muted-foreground" />
  const destination = rule.scope === "mesh" ? "目标节点" : rule.exitNodeId !== "" ? "互联网" : rule.scope === "internet" ? "本地出网" : "目标"
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-1" aria-label="规划的经由顺序">
      <Chip tone="muted">路由</Chip>
      <span className="text-[0.6875rem] text-muted-foreground">来源</span>
      {rule.via.map((id, index) => (
        <Fragment key={`${index}-${id}`}>
          {step}
          <NodeChip nodes={nodes} id={id} />
        </Fragment>
      ))}
      {rule.exitNodeId !== "" ? (
        <>
          {step}
          <Chip>
            <GlobeIcon className="size-3 text-brand" />
            {nodeRefLabel(resolveNode(nodes, rule.exitNodeId))} 出口
          </Chip>
        </>
      ) : null}
      {step}
      <span className="text-[0.6875rem] text-muted-foreground">{destination}</span>
    </span>
  )
}

/** One-line readable headline, e.g. "nyc 访问 互联网 example.com · 经 lax，从 tyo 出口". */
export function ruleHeadline(rule: RuleDraft, nodes: NodeSummary[]): string {
  const countries = parseCountryList(rule.sourceCountries)
  const names = rule.sourceNodeIds.map((id) => (id ? nodeRefLabel(resolveNode(nodes, id)) : "未选择"))
  const source =
    names.length === 0 && countries.length === 0
      ? "任意来源"
      : [names.slice(0, 2).join("、") + (names.length > 2 ? ` 等 ${names.length} 个节点` : ""), countries.length > 0 ? `国家 ${countries.slice(0, 3).join("、")} 的节点` : ""]
          .filter((part) => part !== "")
          .join(" 且 ")
  const target =
    rule.destinations.length === 0
      ? `全部${scopeLabels[rule.scope]}`
      : rule.destinations.length === 1 && rule.destinations[0]
        ? `${scopeLabels[rule.scope]} ${rule.destinations[0].kind === "node" ? nodeRefLabel(resolveNode(nodes, rule.destinations[0].value)) : rule.destinations[0].value || "—"}`
        : `${scopeLabels[rule.scope]}（${rule.destinations.length} 个条件之一）`
  // The action itself is shown as a badge next to the headline.
  if (rule.action === "reject") return `${source} 访问 ${target}`
  const via = rule.via.length > 0 ? `经 ${rule.via.map((id) => (id ? nodeRefLabel(resolveNode(nodes, id)) : "未选择")).join(" › ")}` : ""
  const exit = rule.exitNodeId !== "" ? `从 ${nodeRefLabel(resolveNode(nodes, rule.exitNodeId))} 出口` : ""
  const path = [via, exit].filter((part) => part !== "").join("，")
  return `${source} 访问 ${target}${path ? ` · ${path}` : rule.scope === "mesh" ? " · 直达" : rule.scope === "internet" ? " · 本地出网" : ""}`
}
