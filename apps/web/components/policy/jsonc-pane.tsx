"use client"

import { SparklesIcon } from "lucide-react"
import { useId, useRef, type ReactNode } from "react"

import { PolicyCodeEditor, type CodeEditorHandle } from "@/components/policy/code-editor"
import { ProblemList } from "@/components/policy/problem-list"
import { Button } from "@/components/ui/button"
import { Kbd } from "@/components/ui/kbd"
import type { ReferenceSuggestions } from "@/lib/jsonc-complete"
import type { DocumentProblem } from "@/lib/policy-document"

/**
 * JSONC view of a policy draft: the code editor, a toolbar, and the full
 * problem list. Shared by every policy editor; it holds no state of its own
 * beyond the editor handle, so the draft stays in one place.
 */
export function JsoncPane({
  label,
  text,
  problems,
  onChange,
  schemaRoot,
  schema,
  references,
  readOnly,
  notice,
}: {
  label: string
  text: string
  problems: readonly DocumentProblem[]
  onChange: (text: string) => void
  schemaRoot: unknown
  schema: unknown
  references?: ReferenceSuggestions
  readOnly?: boolean
  notice?: ReactNode
}) {
  const handle = useRef<CodeEditorHandle | null>(null)
  const problemsId = useId()
  return (
    <div className="flex flex-col" data-testid="jsonc-pane">
      <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2 text-xs text-muted-foreground">
        <span className="min-w-0 flex-1">
          支持 <code className="font-mono">//</code> 与 <code className="font-mono">/* */</code> 注释和尾随逗号；字段名、枚举值按 controller
          schema 补全（<Kbd>Ctrl</Kbd> + <Kbd>Space</Kbd>）。
        </span>
        <Button type="button" variant="ghost" size="sm" disabled={readOnly} onClick={() => handle.current?.complete()}>
          <SparklesIcon data-icon="inline-start" />
          补全
        </Button>
      </div>
      {notice ? <div className="border-b px-3 py-2 text-xs text-muted-foreground">{notice}</div> : null}
      <div className="h-[32rem] min-h-0 overflow-hidden">
        <PolicyCodeEditor
          value={text}
          onChange={onChange}
          problems={problems}
          problemsText={text}
          schemaRoot={schemaRoot}
          schema={schema}
          references={references}
          readOnly={readOnly}
          label={label}
          describedBy={problemsId}
          handleRef={(next) => {
            handle.current = next
          }}
        />
      </div>
      <ProblemList
        id={problemsId}
        problems={problems}
        className="border-t"
        onSelect={(problem) => handle.current?.focusAt(problem.from, problem.to)}
      />
    </div>
  )
}
