"use client"

import { CircleAlertIcon } from "lucide-react"

import type { DocumentProblem } from "@/lib/policy-document"
import { cn } from "@/lib/utils"

const sourceLabels: Record<DocumentProblem["source"], string> = {
  syntax: "JSONC 语法",
  schema: "Schema",
  semantic: "编辑器检查",
}

function pathLabel(path: readonly (string | number)[]): string {
  return path.reduce<string>(
    (label, segment) => (typeof segment === "number" ? `${label}[${segment}]` : label ? `${label}.${segment}` : segment),
    ""
  )
}

/**
 * Every problem of the JSONC buffer as a plain list, so they are reachable
 * without hovering editor underlines. Selecting one moves the cursor there.
 */
export function ProblemList({
  id,
  problems,
  onSelect,
  className,
}: {
  id?: string
  problems: readonly DocumentProblem[]
  onSelect: (problem: DocumentProblem) => void
  className?: string
}) {
  if (problems.length === 0) {
    return (
      <p id={id} role="status" className={cn("px-3 py-2 text-xs text-muted-foreground", className)}>
        没有语法或 schema 问题。
      </p>
    )
  }
  return (
    <div id={id} className={cn("flex flex-col", className)}>
      <p role="status" className="px-3 pt-2 text-xs font-medium text-destructive">
        {problems.length} 个问题
      </p>
      <ul aria-label="JSONC 问题" className="flex max-h-48 flex-col overflow-y-auto py-1">
        {problems.map((problem, index) => (
          <li key={`${problem.from}-${index}`}>
            <button
              type="button"
              onClick={() => onSelect(problem)}
              className="flex w-full items-start gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
            >
              <CircleAlertIcon aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-destructive" />
              <span className="shrink-0 font-mono text-muted-foreground tabular-nums">
                {problem.line}:{problem.column}
              </span>
              <span className="min-w-0 flex-1">
                <span className="break-words">{problem.message}</span>
                <span className="block text-[0.6875rem] text-muted-foreground">
                  {sourceLabels[problem.source]}
                  {problem.path.length > 0 ? ` · ${pathLabel(problem.path)}` : ""}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}
