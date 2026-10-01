"use client"

import type { ConnectionNode, MeshAccessRule, TagSnapshot } from "@meshway/control-client"
import {
  ArrowDownIcon,
  ArrowLeftRightIcon,
  ArrowRightIcon,
  ArrowUpIcon,
  CircleAlertIcon,
  CopyIcon,
  EllipsisIcon,
  PencilIcon,
  Trash2Icon,
} from "lucide-react"
import { useEffect, useRef } from "react"

import { SelectorList } from "@/components/access/selector-chip"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { actionLabels, type RuleChange } from "@/lib/access-model"
import { cn } from "@/lib/utils"

export type RuleCardState = {
  change: RuleChange
  problems: number
  /** 1-based position of an earlier rule that already covers this one. */
  shadowedBy: number | null
}

/**
 * One first-match access rule: who → whom, the direction, and the action.
 * Pure display of the draft; it never evaluates a flow.
 */
export function AccessRuleCard({
  rule,
  index,
  count,
  state,
  inventory,
  tags,
  highlighted,
  disabled,
  onEdit,
  onMove,
  onDuplicate,
  onDelete,
}: {
  rule: MeshAccessRule
  index: number
  count: number
  state: RuleCardState
  inventory: ConnectionNode[]
  tags: TagSnapshot
  highlighted: boolean
  disabled: boolean
  onEdit: () => void
  onMove: (target: number) => void
  onDuplicate: () => void
  onDelete: () => void
}) {
  const ref = useRef<HTMLLIElement>(null)
  useEffect(() => {
    if (highlighted) ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" })
  }, [highlighted])
  const label = rule.id || `#${index + 1}`
  return (
    <li
      ref={ref}
      data-testid="access-rule-card"
      data-rule-id={rule.id}
      aria-label={`规则 ${label}`}
      className={cn(
        "flex gap-3 px-3 py-3 sm:px-4",
        highlighted && "bg-brand/[0.06] ring-1 ring-brand/40 ring-inset"
      )}
    >
      <span
        aria-hidden="true"
        className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md border font-mono text-[0.6875rem] text-muted-foreground tabular-nums"
      >
        {index + 1}
      </span>
      <button
        type="button"
        onClick={onEdit}
        disabled={disabled}
        aria-label={`编辑规则 ${label}`}
        className="flex min-w-0 flex-1 flex-col gap-2 rounded-md text-left focus-visible:ring-2 focus-visible:ring-ring/30 focus-visible:outline-none disabled:cursor-default"
      >
        <span className="flex flex-wrap items-center gap-2">
          <Badge
            variant="outline"
            data-testid="access-rule-action"
            className={rule.action === "allow" ? "border-success/50 text-success" : "border-destructive/50 text-destructive"}
          >
            {actionLabels[rule.action]}
          </Badge>
          <span className="truncate font-mono text-xs text-muted-foreground">{rule.id}</span>
          {state.change !== "unchanged" ? (
            <Badge variant="secondary">{state.change === "new" ? "新增" : "已修改"}</Badge>
          ) : null}
          {state.problems > 0 ? (
            <span className="flex items-center gap-1 text-xs text-destructive">
              <CircleAlertIcon className="size-3.5" />
              {state.problems} 个问题
            </span>
          ) : null}
          {state.shadowedBy !== null ? (
            <span className="text-xs text-warning">被第 {state.shadowedBy} 条完全覆盖，永远不会命中</span>
          ) : null}
        </span>
        <span className="grid min-w-0 grid-cols-1 items-start gap-2 sm:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]">
          <SelectorList list={rule.source} inventory={inventory} tags={tags} label="来源" />
          <span
            className="flex items-center gap-1 self-center text-xs text-muted-foreground"
            data-testid="access-rule-direction"
            title={rule.bidirectional ? "双向：同一位置也匹配反向流量" : "单向：只匹配来源 → 目标"}
          >
            {rule.bidirectional ? (
              <>
                <ArrowLeftRightIcon aria-hidden="true" className="size-4 text-foreground" />
                双向
              </>
            ) : (
              <>
                <ArrowRightIcon aria-hidden="true" className="size-4 text-foreground" />
                单向
              </>
            )}
          </span>
          <SelectorList list={rule.destination} inventory={inventory} tags={tags} label="目标" />
        </span>
      </button>
      <div className="flex shrink-0 items-start gap-0.5">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`上移规则 ${label}`}
          disabled={disabled || index === 0}
          onClick={() => onMove(index - 1)}
        >
          <ArrowUpIcon />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`下移规则 ${label}`}
          disabled={disabled || index === count - 1}
          onClick={() => onMove(index + 1)}
        >
          <ArrowDownIcon />
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={<Button variant="ghost" size="icon-sm" aria-label={`规则 ${label} 的操作`} disabled={disabled} />}
          >
            <EllipsisIcon />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={onEdit}>
              <PencilIcon />
              编辑
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onDuplicate}>
              <CopyIcon />
              复制
            </DropdownMenuItem>
            <DropdownMenuItem disabled={index === 0} onClick={() => onMove(0)}>
              <ArrowUpIcon />
              移到最前
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onClick={onDelete}>
              <Trash2Icon />
              删除
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </li>
  )
}
