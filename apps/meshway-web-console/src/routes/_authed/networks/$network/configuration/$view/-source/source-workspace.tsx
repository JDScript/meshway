import type { NetworkSummary } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import {
  ChevronDownIcon,
  ChevronUpIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  InfoIcon,
  WandSparklesIcon,
} from "lucide-react"
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react"
import { toast } from "sonner"

import { failureText } from "@/api/failure"
import {
  configurationQuery,
  readSource as readSourceText,
  schemaQuery,
} from "@/api/queries/configuration"
import { CloseButton } from "@/components/chrome/close-button"
import { RevisionLine } from "@/components/chrome/draft-mark"
import { IconAction } from "@/components/chrome/icon-action"
import { EmptyLine } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { DraftNotice } from "@/components/configuration/notice"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { Skeleton } from "@/components/ui/skeleton"
import { nodeNamer } from "@/lib/configuration/diagnostics"
import {
  holdsOperations,
  isOutdated,
  needsSource,
  operationsOf,
} from "@/lib/configuration/draft-state"
import {
  formatJsonc,
  lineColumn,
  parseJsonc,
  spanOfPointer,
} from "@/lib/configuration/jsonc"
import { readSource, type SourceProblem } from "@/lib/configuration/problems"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import type { EditorHandle, EditorMark } from "./code-editor"

// CodeMirror loads with the first source opened.
const CodeEditor = lazy(() =>
  import("./code-editor").then((module) => ({ default: module.CodeEditor }))
)

const SEVERITY = {
  syntax: "error",
  schema: "warning",
  controller: "error",
  warning: "warning",
} as const

/**
 * The whole configuration as JSONC, for those who edit it: the draft's
 * text, any text at all (invalid, with comments), kept as typed. Syntax is
 * checked here and stops applying; the schema's hints are marked as you
 * type; the controller's diagnostics from its last check sit at their
 * pointers. Applying is the commit bar's.
 */
export function SourceWorkspace({
  networkId,
  network,
  at,
  checking,
  onApplyDraft,
  onCompare,
  onClose,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
  /** A JSON Pointer to open at. */
  at: string | null
  /** The draft is being checked. */
  checking: boolean
  /** Checks the draft and opens its review, as the commit bar does. */
  onApplyDraft: () => void
  onCompare: (from: number, to: number) => void
  onClose: () => void
}) {
  const editable = network?.actions.editConfiguration ?? false
  const configuration = useQuery(configurationQuery(networkId))
  const current = configuration.data?.revision ?? network?.currentRevision
  const draft = useConfigurationDraft(networkId)
  const entry = draft.entry
  const schema = useQuery({ ...schemaQuery(), enabled: editable })
  const [failed, setFailed] = useState(false)
  const reading = useRef(false)

  // The source is read into the draft only: it holds secrets.
  const load = editable && needsSource(entry, current)
  const { load: loadDraft } = draft
  useEffect(() => {
    if (!load || reading.current) return
    reading.current = true
    setFailed(false)
    const controller = new AbortController()
    readSourceText(networkId, controller.signal)
      .then(loadDraft)
      .catch((error: unknown) => {
        if (controller.signal.aborted) return
        setFailed(true)
        const text = failureText(error)
        if (text) toast.error(text, { id: "configuration-source" })
      })
      .finally(() => {
        reading.current = false
      })
    return () => {
      controller.abort()
      reading.current = false
    }
  }, [load, networkId, current, loadDraft])

  const text = entry?.input.kind === "document" ? entry.input.text : null
  const formEdits = holdsOperations(entry)
  const problems = useMemo(
    () =>
      text === null
        ? null
        : readSource(
            text,
            schema.data ?? null,
            entry?.checked ?? null,
            nodeNamer(configuration.data?.configuration.nodes ?? {})
          ),
    [text, schema.data, entry?.checked, configuration.data]
  )
  const outdated = isOutdated(entry, current)

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2
            id={surfaceTitleId("configuration", "workspace")}
            className={className}
          >
            {m.configuration_source()}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
        meta={
          <RevisionLine
            revision={entry?.baseRevision ?? current ?? null}
            draftBase={draft.dirty && entry ? entry.baseRevision : null}
            facts={
              outdated && entry && current !== undefined
                ? [
                    <span key="moved" className="text-warning">
                      {m.configuration_source_moved({ revision: current })}
                    </span>,
                    <button
                      key="compare"
                      type="button"
                      className="text-foreground underline-offset-2 hover:underline focus-visible:underline focus-visible:outline-none"
                      onClick={() => onCompare(entry.baseRevision, current)}
                    >
                      {m.configuration_source_compare()}
                    </button>,
                  ]
                : []
            }
          />
        }
      />
      <div className="flex min-h-0 flex-1 flex-col px-6 pt-1 pb-6">
        {!editable ? (
          <EmptyLine>{m.configuration_read_denied()}</EmptyLine>
        ) : formEdits ? (
          <DraftNotice
            text={m.configuration_notice_operations({
              count: operationsOf(entry).length,
            })}
            primaryLabel={m.configuration_commit_apply()}
            busy={checking}
            onPrimary={onApplyDraft}
            onDiscard={draft.discard}
          />
        ) : text === null || problems === null ? (
          failed ? (
            <EmptyLine>{m.configuration_source_failed()}</EmptyLine>
          ) : (
            <Skeleton className="min-h-0 flex-1 rounded-row" />
          )
        ) : (
          <SourceEditor
            key={networkId}
            text={text}
            problems={problems}
            at={at}
            checkedText={
              entry?.checked?.input.kind === "document"
                ? entry.checked.input.text
                : null
            }
            onChange={draft.edit}
          />
        )}
      </div>
    </div>
  )
}

function SourceEditor({
  text,
  problems,
  at,
  checkedText,
  onChange,
}: {
  text: string
  problems: NonNullable<ReturnType<typeof readSource>>
  at: string | null
  /** The text the controller last checked, to jump to its first problem once. */
  checkedText: string | null
  onChange: (text: string) => void
}) {
  const [editor, setEditor] = useState<EditorHandle | null>(null)
  const all = useMemo(
    () =>
      [
        ...problems.syntax,
        ...problems.schema,
        ...problems.controller,
        ...problems.warnings,
      ].sort((a, b) => a.from - b.from),
    [problems]
  )
  const marks: EditorMark[] = useMemo(
    () =>
      all.map((problem) => ({
        from: problem.from,
        to: problem.to,
        severity:
          (problem.origin === "controller" || problem.origin === "warning") &&
          problems.stale
            ? "info"
            : SEVERITY[problem.origin],
        message: problem.message,
      })),
    [all, problems.stale]
  )

  // Opened at a pointer: there once, when the editor is ready.
  const jumpedTo = useRef<string | null>(null)
  useEffect(() => {
    if (!editor || at === null || jumpedTo.current === at) return
    jumpedTo.current = at
    const parsed = parseJsonc(text)
    if (!parsed.ok) return
    const span = spanOfPointer(parsed.tree, at)
    if (span) editor.focusAt(span.from, span.to)
  }, [editor, at, text])

  // The controller refused a text: to its first problem, once per check.
  const jumpedFor = useRef<string | null>(null)
  const firstController = problems.stale ? null : problems.controller[0]
  useEffect(() => {
    if (!editor || checkedText === null || !firstController) return
    if (jumpedFor.current === checkedText) return
    jumpedFor.current = checkedText
    editor.focusAt(firstController.from, firstController.to)
  }, [editor, checkedText, firstController])

  const stop = useRef<number | null>(null)
  const jump = (direction: 1 | -1) => {
    if (all.length === 0) return
    const index =
      stop.current === null
        ? direction === 1
          ? 0
          : all.length - 1
        : (stop.current + direction + all.length) % all.length
    stop.current = index
    const problem = all[index]
    if (problem) editor?.focusAt(problem.from, problem.to)
  }

  const first = problems.syntax[0]
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex h-9 shrink-0 items-center gap-2 rounded-full bg-fill-subtle pr-1 pl-3.5 text-row">
        {first ? (
          <button
            type="button"
            onClick={() => editor?.focusAt(first.from, first.to)}
            className="flex min-w-0 flex-1 items-center gap-2 text-left outline-none focus-visible:underline"
          >
            <CircleAlertIcon
              aria-hidden="true"
              className="size-3.5 shrink-0 text-destructive"
            />
            <span className="shrink-0 font-medium">
              {m.configuration_source_syntax({ count: problems.syntax.length })}
            </span>
            <span className="min-w-0 truncate text-muted-foreground">
              {m.configuration_source_problem_at({
                line: lineColumn(text, first.from).line,
                reason: first.message,
              })}
            </span>
          </button>
        ) : (
          <span className="flex min-w-0 flex-1 items-center gap-2">
            <CircleCheckIcon
              aria-hidden="true"
              className="size-3.5 shrink-0 text-success"
            />
            <span className="truncate">{m.configuration_source_valid()}</span>
          </span>
        )}
        <ProblemList
          text={text}
          problems={problems}
          onJump={(problem) => editor?.focusAt(problem.from, problem.to)}
        />
        <span className="flex shrink-0 items-center">
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className="rounded-full"
            disabled={all.length === 0}
            aria-label={m.configuration_source_previous()}
            onClick={() => jump(-1)}
          >
            <ChevronUpIcon />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className="rounded-full"
            disabled={all.length === 0}
            aria-label={m.configuration_source_next()}
            onClick={() => jump(1)}
          >
            <ChevronDownIcon />
          </Button>
          <IconAction
            label={m.configuration_source_format()}
            icon={WandSparklesIcon}
            disabled={!problems.valid}
            onClick={() => onChange(formatJsonc(text))}
            className="size-7"
          />
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
            readOnly={false}
            label={m.configuration_source_editor()}
            handle={setEditor}
          />
        </Suspense>
      </div>
    </div>
  )
}

/** The counts of schema hints and controller diagnostics; every problem listed, each one a jump. */
function ProblemList({
  text,
  problems,
  onJump,
}: {
  text: string
  problems: NonNullable<ReturnType<typeof readSource>>
  onJump: (problem: SourceProblem) => void
}) {
  const [open, setOpen] = useState(false)
  const listed = [
    ...problems.controller,
    ...problems.warnings,
    ...problems.schema,
  ]
  const schemaCount = problems.schema.length
  const controllerCount = problems.controller.length
  const warningCount = problems.warnings.length
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        disabled={listed.length === 0}
        className="flex shrink-0 items-center gap-2 rounded-full px-2 py-1 text-meta tabular-nums outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none"
      >
        {controllerCount > 0 ? (
          <span
            className={cn(
              "inline-flex items-center gap-1",
              problems.stale ? "text-muted-foreground" : "text-destructive"
            )}
          >
            <CircleAlertIcon aria-hidden="true" className="size-3" />
            {problems.stale
              ? m.configuration_source_checked_earlier({
                  count: controllerCount,
                })
              : m.configuration_source_controller({ count: controllerCount })}
          </span>
        ) : null}
        {warningCount > 0 ? (
          <span
            className={cn(
              "inline-flex items-center gap-1",
              problems.stale ? "text-muted-foreground" : "text-warning"
            )}
          >
            <CircleAlertIcon aria-hidden="true" className="size-3" />
            {m.configuration_source_warnings({ count: warningCount })}
          </span>
        ) : null}
        {schemaCount > 0 ? (
          <span className="inline-flex items-center gap-1 text-warning">
            <InfoIcon aria-hidden="true" className="size-3" />
            {m.configuration_source_schema({ count: schemaCount })}
          </span>
        ) : null}
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="glass w-96 gap-1 rounded-row p-1.5 text-foreground ring-0"
        collisionPadding={16}
      >
        <div className="px-2.5 pt-1.5 pb-1 text-meta text-muted-foreground">
          {m.configuration_problems_title()}
        </div>
        <ul className="flex max-h-80 flex-col gap-0.5 overflow-y-auto">
          {listed.map((problem, index) => (
            <li key={index}>
              <button
                type="button"
                className="flex w-full flex-col gap-0.5 rounded-tile px-2.5 py-2 text-left outline-none hover:bg-fill-hover focus-visible:bg-fill-hover"
                onClick={() => {
                  setOpen(false)
                  onJump(problem)
                }}
              >
                <span className="flex min-w-0 items-center gap-1.5 text-meta text-muted-foreground">
                  {problem.origin === "schema" ? (
                    <InfoIcon
                      aria-hidden="true"
                      className="size-3 shrink-0 text-warning"
                    />
                  ) : (
                    <CircleAlertIcon
                      aria-hidden="true"
                      className={cn(
                        "size-3 shrink-0",
                        problems.stale
                          ? undefined
                          : problem.origin === "warning"
                            ? "text-warning"
                            : "text-destructive"
                      )}
                    />
                  )}
                  <span className="shrink-0 tabular-nums">
                    {m.configuration_source_line({
                      line: lineColumn(text, problem.from).line,
                    })}
                  </span>
                  <code className="min-w-0 truncate font-mono">
                    {problem.path || m.configuration_problem_root()}
                  </code>
                </span>
                <span className="text-row">{problem.message}</span>
              </button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
