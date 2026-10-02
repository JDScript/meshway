import { cn } from "cn"
import {
  ChevronDownIcon,
  ChevronUpIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  InfoIcon,
} from "lucide-react"
import { lazy, Suspense, useMemo, useRef } from "react"

import { Button } from "@/components/ui/button"
import { m } from "@/paraglide/messages.js"

import type { EditorHandle, EditorMark } from "./code-editor"
import { locate, type PlacedDiagnostic, type PolicyDocument } from "./document"
import { findNearest, lineColumn } from "./jsonc"
import { problemReason } from "./problems"
import { isLint } from "./rules"
import { withNodeNames, type SelectorNames } from "./selectors"

// The editor (CodeMirror) loads with the first source workspace opened.
const CodeEditor = lazy(() =>
  import("./code-editor").then((module) => ({ default: module.CodeEditor }))
)

/** The controller's diagnostics on the text they can be placed on. */
function placedMarks(
  document: PolicyDocument,
  diagnostics: readonly PlacedDiagnostic[],
  names: SelectorNames
): EditorMark[] {
  const { tree, policy } = document
  if (!tree || !policy) return []
  const marks: EditorMark[] = []
  for (const diagnostic of diagnostics) {
    if (diagnostic.placed.kind !== "field") continue
    const path = locate(policy, diagnostic.placed.anchor)
    if (!path) continue
    const { node } = findNearest(tree, path)
    marks.push({
      from: node.offset,
      to: node.offset + node.length,
      severity: isLint(diagnostic) ? "warning" : "error",
      message: withNodeNames(diagnostic.message, names),
    })
  }
  return marks
}

/** The whole policy as JSONC: the same draft the graphical layers edit. */
export function SourceEditor({
  document,
  diagnostics,
  names,
  editable,
  onChange,
}: {
  document: PolicyDocument
  diagnostics: readonly PlacedDiagnostic[]
  names: SelectorNames
  editable: boolean
  onChange: (text: string) => void
}) {
  const editor = useRef<EditorHandle | null>(null)
  const text = document.text
  const controller = useMemo(
    () => placedMarks(document, diagnostics, names),
    [document, diagnostics, names]
  )
  const marks: EditorMark[] = useMemo(
    () => [
      ...document.problems.map((problem) => ({
        from: problem.from,
        to: problem.to,
        severity: "error" as const,
        message: problemReason(problem),
      })),
      ...controller,
    ],
    [document.problems, controller]
  )
  const stops = useMemo(
    () => [...marks].sort((a, b) => a.from - b.from),
    [marks]
  )
  const first = document.problems[0]
  const lints = controller.filter((mark) => mark.severity === "warning").length
  const errors = controller.length - lints

  const currentStop = useRef<number | null>(null)
  const jump = (direction: 1 | -1) => {
    if (stops.length === 0) return
    const at = currentStop.current
    const next =
      at === null
        ? direction === 1
          ? 0
          : stops.length - 1
        : (at + direction + stops.length) % stops.length
    currentStop.current = next
    const stop = stops[next]
    if (stop) editor.current?.focusAt(stop.from, stop.to)
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex h-9 shrink-0 items-center gap-2 rounded-full bg-fill-subtle pr-1 pl-3.5 text-row">
        {first ? (
          <button
            type="button"
            onClick={() => editor.current?.focusAt(first.from, first.to)}
            className="flex min-w-0 flex-1 items-center gap-2 text-left outline-none focus-visible:underline"
          >
            <CircleAlertIcon
              aria-hidden="true"
              className="size-3.5 shrink-0 text-destructive"
            />
            <span className="shrink-0 font-medium">
              {m.policy_count_problems({ count: document.problems.length })}
            </span>
            <span className="min-w-0 truncate text-muted-foreground">
              {m.policy_problem_at({
                line: lineColumn(text, first.from).line,
                reason: problemReason(first),
              })}
            </span>
          </button>
        ) : (
          <span className="flex min-w-0 flex-1 items-center gap-2">
            <CircleCheckIcon
              aria-hidden="true"
              className="size-3.5 shrink-0 text-success"
            />
            <span className="truncate">{m.policy_source_valid()}</span>
          </span>
        )}
        <span className="flex shrink-0 items-center gap-2 text-meta tabular-nums">
          {errors > 0 ? (
            <span className="inline-flex items-center gap-1 text-destructive">
              <CircleAlertIcon aria-hidden="true" className="size-3" />
              {m.policy_source_controller({ count: errors })}
            </span>
          ) : null}
          {lints > 0 ? (
            <span className="inline-flex items-center gap-1 text-warning">
              <InfoIcon aria-hidden="true" className="size-3" />
              {m.policy_count_lints({ count: lints })}
            </span>
          ) : null}
        </span>
        <span className="flex shrink-0 items-center">
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className="rounded-full"
            disabled={stops.length === 0}
            aria-label={m.policy_source_previous()}
            onClick={() => jump(-1)}
          >
            <ChevronUpIcon />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className="rounded-full"
            disabled={stops.length === 0}
            aria-label={m.policy_source_next()}
            onClick={() => jump(1)}
          >
            <ChevronDownIcon />
          </Button>
        </span>
      </div>
      <div
        className={cn(
          "min-h-0 flex-1 overflow-hidden rounded-row bg-field ring-1 ring-divider transition-shadow ring-inset focus-within:ring-ring/60",
          first && "ring-destructive/40"
        )}
      >
        <Suspense fallback={null}>
          <CodeEditor
            value={text}
            onChange={onChange}
            marks={marks}
            marksFor={text}
            readOnly={!editable}
            label={m.policy_layer_source()}
            handle={(handle) => {
              editor.current = handle
            }}
          />
        </Suspense>
      </div>
    </div>
  )
}
