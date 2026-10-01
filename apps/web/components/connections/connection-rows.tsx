"use client"

import type { ConnectionNode, ConnectionNeighborObservation } from "@meshway/control-client"
import { ArrowRightIcon, CircleAlertIcon, PlusIcon, XIcon } from "lucide-react"
import { useEffect, useRef, type ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { formatRtt, neighborLabel } from "@/lib/connection-runtime-model"
import {
  endpointLabel,
  nodeName,
  resolveConnectionNode,
  transportLabels,
  type CandidateDraft,
  type ConnectionIssue,
  type ItemChange,
  type ListenerDraft,
} from "@/lib/connections-model"
import { cn } from "@/lib/utils"

const changeBadge: Record<ItemChange, { label: string; className: string } | null> = {
  new: { label: "新增", className: "border-brand/40 text-brand" },
  modified: { label: "已修改", className: "border-warning/50 text-warning" },
  unchanged: null,
}

function useScrollIntoView(highlighted: boolean) {
  const ref = useRef<HTMLLIElement>(null)
  useEffect(() => {
    if (highlighted) ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" })
  }, [highlighted])
  return ref
}

function RowBadges({ change, problems }: { change: ItemChange; problems: number }) {
  const badge = changeBadge[change]
  return (
    <>
      {badge ? (
        <Badge variant="outline" className={badge.className}>
          {badge.label}
        </Badge>
      ) : null}
      {problems > 0 ? (
        <Badge variant="destructive">
          <CircleAlertIcon data-icon="inline-start" />
          {problems} 个问题
        </Badge>
      ) : null}
    </>
  )
}

const rowClass = (highlighted: boolean, problems: number) =>
  cn(
    "group/row relative flex items-start gap-3 px-3 py-3 transition-colors hover:bg-muted/30 sm:px-4",
    highlighted && "bg-brand/[0.06] ring-1 ring-brand/40 ring-inset",
    problems > 0 && "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-destructive"
  )

function Secondary({ children }: { children: ReactNode }) {
  return <span className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">{children}</span>
}

function Item({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex min-w-0 max-w-full items-baseline gap-1.5">
      <span className="shrink-0">{label}</span>
      <span className="min-w-0 truncate text-foreground">{children}</span>
    </span>
  )
}

/** Saved-source diagnostics only describe an unchanged item. */
export function problemCount(change: ItemChange, issues: ConnectionIssue[], diagnostics: number) {
  return issues.length + (change === "unchanged" ? diagnostics : 0)
}

export type ListenerRuntime = { state: "bound" | "error"; reason: string } | null

export function ListenerRow({
  listener,
  nodes,
  showOwner,
  change,
  problems,
  dialers,
  runtime,
  highlighted,
  disabled,
  onEdit,
  onDial,
  onDelete,
}: {
  listener: ListenerDraft
  nodes: ConnectionNode[]
  /** Global view: lead with the owning node. */
  showOwner: boolean
  change: ItemChange
  problems: number
  /** Names of nodes that dial this listener in the draft. */
  dialers: string[]
  /** Fresh self-reported listener state, or null. */
  runtime: ListenerRuntime
  highlighted: boolean
  disabled: boolean
  onEdit: () => void
  onDial: () => void
  onDelete: () => void
}) {
  const ref = useScrollIntoView(highlighted)
  const label = listener.id.trim() || "（空 ID）"
  const owner = resolveConnectionNode(nodes, listener.nodeId)
  const endpoints = listener.endpoints.filter((endpoint) => endpoint.host.trim() !== "")
  return (
    <li
      ref={ref}
      data-testid="listener-row"
      data-listener-id={listener.id.trim()}
      aria-label={`监听器 ${label}`}
      className={rowClass(highlighted, problems)}
    >
      <button
        type="button"
        onClick={onEdit}
        disabled={disabled}
        aria-label={`编辑监听器 ${label}`}
        className="flex min-w-0 flex-1 flex-col gap-1.5 text-left focus-visible:outline-none disabled:cursor-not-allowed"
      >
        <span className="flex min-w-0 flex-wrap items-center gap-2">
          {showOwner ? (
            <span className={cn("truncate text-sm font-medium group-hover/row:underline", owner.kind !== "active" && "text-destructive")}>
              {nodeName(nodes, listener.nodeId)}
            </span>
          ) : null}
          <Badge variant="secondary" className="font-normal">
            {transportLabels[listener.transport]}
          </Badge>
          {!showOwner ? (
            <span className="truncate text-sm font-medium group-hover/row:underline">
              {endpoints.length > 0 ? endpointLabel(endpoints[0]?.host ?? "", endpoints[0]?.port ?? "") : "未填写对外地址"}
            </span>
          ) : null}
          <RowBadges change={change} problems={problems} />
          {runtime ? (
            <Badge variant="outline" className={cn("font-normal", runtime.state === "bound" ? "text-success" : "text-destructive")}>
              {runtime.state === "bound" ? "已监听" : `监听失败${runtime.reason ? `：${runtime.reason}` : ""}`}
            </Badge>
          ) : null}
        </span>
        <Secondary>
          <Item label="其他节点通过">
            <span className="font-mono">
              {endpoints.length === 0
                ? "—"
                : endpoints.map((endpoint) => endpointLabel(endpoint.host, endpoint.port)).join("、")}
            </span>
          </Item>
          <Item label="本机监听">
            <code className="font-mono">{listener.bind.trim() || "—"}</code>
          </Item>
        </Secondary>
        <Secondary>
          <span>{dialers.length > 0 ? `由 ${dialers.join("、")} 主动连接` : "还没有节点连接到这里"}</span>
          {listener.credentialRef.trim() !== "" ? (
            <Item label="配置档">
              <code className="font-mono">{listener.credentialRef.trim()}</code>
            </Item>
          ) : null}
          <span className="font-mono text-[0.6875rem]">ID {label}</span>
        </Secondary>
      </button>
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={disabled || listener.id.trim() === ""}
          onClick={onDial}
          aria-label={`为监听器 ${label} 添加拨号候选`}
          title="让另一个节点主动连接到这里"
        >
          <PlusIcon data-icon="inline-start" />
          <span className="hidden sm:inline">允许连接</span>
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          onClick={onDelete}
          aria-label={`删除监听器 ${label}`}
          className="text-muted-foreground hover:text-destructive"
        >
          <XIcon />
        </Button>
      </div>
    </li>
  )
}

export function CandidateRow({
  candidate,
  listener,
  nodes,
  change,
  problems,
  role,
  neighbors,
  highlighted,
  disabled,
  onEdit,
  onDelete,
}: {
  candidate: CandidateDraft
  listener: ListenerDraft | undefined
  nodes: ConnectionNode[]
  change: ItemChange
  problems: number
  /** Primary / backup label within the dialer → acceptor pair. */
  role: string | null
  /** Fresh self-reported neighbors for this candidate. */
  neighbors: ConnectionNeighborObservation[]
  highlighted: boolean
  disabled: boolean
  onEdit: () => void
  onDelete: () => void
}) {
  const ref = useScrollIntoView(highlighted)
  const label = candidate.id.trim() || "（空 ID）"
  const dialer = resolveConnectionNode(nodes, candidate.dialerNodeId)
  const acceptor = listener ? resolveConnectionNode(nodes, listener.nodeId) : null
  const neighbor = neighbors[0]
  return (
    <li
      ref={ref}
      data-testid="connection-candidate-row"
      data-candidate-id={candidate.id.trim()}
      aria-label={`拨号候选 ${label}`}
      className={rowClass(highlighted, problems)}
    >
      <button
        type="button"
        onClick={onEdit}
        disabled={disabled}
        aria-label={`编辑拨号候选 ${label}`}
        className="flex min-w-0 flex-1 flex-col gap-1.5 text-left focus-visible:outline-none disabled:cursor-not-allowed"
      >
        <span className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="flex min-w-0 items-center gap-1.5 text-sm font-medium group-hover/row:underline">
            <span className={cn("truncate", dialer.kind !== "active" && "text-destructive")}>
              {nodeName(nodes, candidate.dialerNodeId)}
            </span>
            <ArrowRightIcon className="size-3.5 shrink-0 text-muted-foreground" aria-label="主动连接" />
            <span className={cn("truncate", (!acceptor || acceptor.kind !== "active") && "text-destructive")}>
              {listener ? nodeName(nodes, listener.nodeId) : `未知监听器 ${candidate.listenerId || "—"}`}
            </span>
          </span>
          {listener ? (
            <Badge variant="secondary" className="font-normal">
              {transportLabels[listener.transport]}
            </Badge>
          ) : null}
          {role ? (
            <Badge variant="outline" className="font-normal">
              {role}
            </Badge>
          ) : null}
          <RowBadges change={change} problems={problems} />
          {neighbor ? (
            <Badge
              variant="outline"
              className={cn(
                "font-normal",
                neighbor.state === "authenticated" ? "text-success" : neighbor.state === "unreachable" ? "text-destructive" : ""
              )}
              title="节点 agent 自报的邻居认证状态"
            >
              {neighborLabel(neighbor)}
              {neighbor.state === "authenticated" ? ` · ${formatRtt(neighbor)}` : ""}
            </Badge>
          ) : null}
        </span>
        <Secondary>
          <Item label="连接到">
            <span className="font-mono">
              {candidate.host.trim() !== "" ? endpointLabel(candidate.host, candidate.port) : "未选择入口"}
            </span>
          </Item>
          <Item label="优先级">
            <span className="font-mono tabular-nums">{candidate.priority.trim() || "—"}</span>
          </Item>
          <span className="font-mono text-[0.6875rem]">ID {label}</span>
        </Secondary>
      </button>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        disabled={disabled}
        onClick={onDelete}
        aria-label={`删除拨号候选 ${label}`}
        className="shrink-0 text-muted-foreground hover:text-destructive"
      >
        <XIcon />
      </Button>
    </li>
  )
}
