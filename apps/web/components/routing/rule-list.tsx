"use client"

import type { NodeSummary } from "@meshway/control-client"
import {
  ArrowDownIcon,
  ArrowDownToLineIcon,
  ArrowUpIcon,
  ArrowUpToLineIcon,
  CircleAlertIcon,
  CopyPlusIcon,
  EllipsisIcon,
  PencilIcon,
  Trash2Icon,
} from "lucide-react"
import { useEffect, useRef } from "react"

import { RuleSummary } from "@/components/routing/rule-summary"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import type { RuleChange, RuleDraft } from "@/lib/policy-model"
import { cn } from "@/lib/utils"

export type RuleRowState = {
  change: RuleChange
  /** Local issues that block saving. */
  issues: number
  /** Controller diagnostics for this rule ID. */
  diagnostics: number
  diagnosticsStale: boolean
  /** A catch-all rule above makes this rule unreachable. */
  shadowedBy: string | null
}

export type RuleListActions = {
  onEdit: (index: number) => void
  onMove: (index: number, target: number) => void
  onDuplicate: (index: number) => void
  onDelete: (index: number) => void
}

const changeBadge: Record<RuleChange, { label: string; className: string } | null> = {
  new: { label: "新增", className: "border-brand/40 text-brand" },
  modified: { label: "已修改", className: "border-warning/50 text-warning" },
  unchanged: null,
}

export function RuleList({
  rules,
  states,
  nodes,
  highlightKey,
  disabled,
  actions,
}: {
  rules: RuleDraft[]
  states: RuleRowState[]
  nodes: NodeSummary[]
  highlightKey: string | null
  disabled: boolean
  actions: RuleListActions
}) {
  return (
    <ol aria-label="路由规则（按匹配顺序）" className="divide-y">
      {rules.map((rule, index) => {
        const state = states[index]
        if (!state) return null
        return (
          <RuleRow
            key={rule.key}
            rule={rule}
            index={index}
            count={rules.length}
            state={state}
            nodes={nodes}
            highlighted={highlightKey === rule.key}
            disabled={disabled}
            actions={actions}
          />
        )
      })}
    </ol>
  )
}

function RuleRow({
  rule,
  index,
  count,
  state,
  nodes,
  highlighted,
  disabled,
  actions,
}: {
  rule: RuleDraft
  index: number
  count: number
  state: RuleRowState
  nodes: NodeSummary[]
  highlighted: boolean
  disabled: boolean
  actions: RuleListActions
}) {
  const ref = useRef<HTMLLIElement>(null)
  const label = rule.id.trim() || `#${index + 1}`
  const change = changeBadge[state.change]
  const problems = state.issues + (state.diagnosticsStale ? 0 : state.diagnostics)

  useEffect(() => {
    if (highlighted) ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" })
  }, [highlighted])

  return (
    <li
      ref={ref}
      data-testid="policy-rule"
      aria-label={`规则 ${label}`}
      className={cn(
        "group/rule relative flex items-start gap-3 px-3 py-3 transition-colors hover:bg-muted/30 sm:px-4",
        highlighted && "bg-brand/[0.06] ring-1 ring-brand/40 ring-inset",
        problems > 0 && "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-destructive"
      )}
    >
      <span className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md border bg-background font-mono text-[0.6875rem] text-muted-foreground tabular-nums">
        {index + 1}
      </span>
      <button
        type="button"
        onClick={() => actions.onEdit(index)}
        disabled={disabled}
        aria-label={`编辑规则 ${label}`}
        className="flex min-w-0 flex-1 flex-col gap-2 text-left focus-visible:outline-none disabled:cursor-not-allowed"
      >
        <span className="flex flex-wrap items-center gap-2">
          <span className="truncate font-mono text-sm font-medium group-hover/rule:underline group-focus-within/rule:underline">
            {label}
          </span>
          {change ? (
            <Badge variant="outline" className={change.className}>
              {change.label}
            </Badge>
          ) : null}
          {problems > 0 ? (
            <Badge variant="destructive">
              <CircleAlertIcon data-icon="inline-start" />
              {problems} 个问题
            </Badge>
          ) : state.diagnostics > 0 ? (
            <Badge variant="outline" className="text-muted-foreground">
              上次编译 {state.diagnostics} 个问题
            </Badge>
          ) : null}
          {state.shadowedBy !== null ? (
            <Badge variant="outline" className="text-warning">
              被上方 {state.shadowedBy} 遮蔽
            </Badge>
          ) : null}
        </span>
        <RuleSummary rule={rule} nodes={nodes} />
      </button>
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={`上移规则 ${label}`}
          disabled={disabled || index === 0}
          onClick={() => actions.onMove(index, index - 1)}
          className="hidden sm:inline-flex"
        >
          <ArrowUpIcon />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={`下移规则 ${label}`}
          disabled={disabled || index === count - 1}
          onClick={() => actions.onMove(index, index + 1)}
          className="hidden sm:inline-flex"
        >
          <ArrowDownIcon />
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                disabled={disabled}
                aria-label={`规则 ${label} 的操作`}
              />
            }
          >
            <EllipsisIcon />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-40">
            <DropdownMenuItem onClick={() => actions.onEdit(index)}>
              <PencilIcon />
              编辑
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => actions.onDuplicate(index)}>
              <CopyPlusIcon />
              复制为新规则
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem disabled={index === 0} onClick={() => actions.onMove(index, index - 1)}>
              <ArrowUpIcon />
              上移
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={index === count - 1}
              onClick={() => actions.onMove(index, index + 1)}
            >
              <ArrowDownIcon />
              下移
            </DropdownMenuItem>
            <DropdownMenuItem disabled={index === 0} onClick={() => actions.onMove(index, 0)}>
              <ArrowUpToLineIcon />
              移到最前
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={index === count - 1}
              onClick={() => actions.onMove(index, count - 1)}
            >
              <ArrowDownToLineIcon />
              移到最后
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onClick={() => actions.onDelete(index)}>
              <Trash2Icon />
              删除
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </li>
  )
}
