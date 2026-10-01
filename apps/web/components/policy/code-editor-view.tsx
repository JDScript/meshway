"use client"

import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  startCompletion,
  type Completion as CmCompletion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete"
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands"
import { json } from "@codemirror/lang-json"
import { bracketMatching, HighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language"
import { lintGutter, lintKeymap, setDiagnostics, type Diagnostic } from "@codemirror/lint"
import { Annotation, Compartment, EditorState, RangeSetBuilder } from "@codemirror/state"
import {
  Decoration,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  ViewPlugin,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view"
import { tags } from "@lezer/highlight"
import { useEffect, useRef } from "react"

import type { CodeEditorProps } from "@/components/policy/code-editor"
import { commentRanges } from "@/lib/jsonc"
import { completeAt } from "@/lib/jsonc-complete"

/** Marks a programmatic replacement, which must not echo back as an edit. */
const external = Annotation.define<boolean>()

const theme = EditorView.theme({
  "&": {
    height: "100%",
    fontSize: "0.75rem",
    color: "var(--foreground)",
    backgroundColor: "var(--background)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono), ui-monospace, monospace", lineHeight: "1.6" },
  ".cm-content": { caretColor: "var(--foreground)", padding: "0.5rem 0" },
  ".cm-gutters": {
    backgroundColor: "var(--muted)",
    color: "var(--muted-foreground)",
    borderRight: "1px solid var(--border)",
  },
  ".cm-activeLine": { backgroundColor: "color-mix(in oklch, var(--accent) 60%, transparent)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--accent)", color: "var(--foreground)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "color-mix(in oklch, var(--brand) 25%, transparent)",
  },
  ".cm-tooltip": {
    backgroundColor: "var(--popover)",
    color: "var(--popover-foreground)",
    border: "1px solid var(--border)",
    borderRadius: "calc(var(--radius) - 2px)",
  },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "var(--accent)",
    color: "var(--accent-foreground)",
  },
  ".cm-completionDetail": { color: "var(--muted-foreground)", marginLeft: "0.75rem", fontStyle: "normal" },
  ".cm-diagnostic-error": { borderLeftColor: "var(--destructive)" },
  ".cm-lintRange-error": {
    backgroundImage: "none",
    textDecoration: "underline wavy var(--destructive)",
    textUnderlineOffset: "3px",
  },
  ".cm-jsonc-comment": { color: "var(--muted-foreground)", fontStyle: "italic" },
})

const highlight = HighlightStyle.define([
  { tag: tags.propertyName, color: "var(--brand)" },
  { tag: tags.string, color: "var(--success)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--warning)" },
  { tag: tags.punctuation, color: "var(--muted-foreground)" },
])

/** `@lezer/json` has no comment syntax: comments are styled from the JSONC scanner. */
const comments = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet

    constructor(view: EditorView) {
      this.decorations = scanComments(view)
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) this.decorations = scanComments(update.view)
    }
  },
  { decorations: (plugin) => plugin.decorations }
)

const commentMark = Decoration.mark({ class: "cm-jsonc-comment" })

function scanComments(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>()
  for (const range of commentRanges(view.state.doc.toString())) builder.add(range.from, range.to, commentMark)
  return builder.finish()
}

const completionTypes = { property: "property", enum: "enum", literal: "keyword", reference: "variable" } as const

/** Smallest single replacement turning `from` into `to`, so the cursor survives. */
function minimalChange(from: string, to: string) {
  let start = 0
  while (start < from.length && start < to.length && from[start] === to[start]) start += 1
  let end = 0
  while (
    end < from.length - start &&
    end < to.length - start &&
    from[from.length - 1 - end] === to[to.length - 1 - end]
  ) {
    end += 1
  }
  return { from: start, to: from.length - end, insert: to.slice(start, to.length - end) }
}

export function CodeEditorView({
  value,
  onChange,
  problems,
  problemsText,
  schemaRoot,
  schema,
  references,
  readOnly = false,
  label,
  describedBy,
  handleRef,
}: CodeEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const latest = useRef({ onChange, schemaRoot, schema, references })
  const editable = useRef(new Compartment())
  const attributes = useRef(new Compartment())

  useEffect(() => {
    latest.current = { onChange, schemaRoot, schema, references }
  })

  useEffect(() => {
    const parent = host.current
    if (!parent) return
    const complete = (context: CompletionContext): CompletionResult | null => {
      const { schemaRoot: root, schema: target, references: refs } = latest.current
      const result = completeAt(context.state.doc.toString(), context.pos, root, target, refs)
      if (!result || result.items.length === 0) return null
      if (!context.explicit && result.from === result.to && !context.matchBefore(/[[{,:]\s*$/)) return null
      const options: CmCompletion[] = result.items.map((item) => ({
        label: item.label,
        apply: item.insert,
        detail: item.detail,
        type: completionTypes[item.kind],
      }))
      return { from: result.from, to: result.to, options, filter: false }
    }
    const created = new EditorView({
      parent,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          highlightActiveLine(),
          history(),
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          json(),
          syntaxHighlighting(highlight),
          comments,
          lintGutter(),
          autocompletion({ override: [complete], activateOnTyping: true, icons: false }),
          keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...completionKeymap, ...lintKeymap]),
          theme,
          EditorState.tabSize.of(2),
          editable.current.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
          attributes.current.of(
            EditorView.contentAttributes.of({ "aria-label": label, ...(describedBy ? { "aria-describedby": describedBy } : {}) })
          ),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged) return
            if (update.transactions.some((transaction) => transaction.annotation(external))) return
            latest.current.onChange(update.state.doc.toString())
          }),
        ],
      }),
    })
    view.current = created
    handleRef?.({
      focusAt(from, to = from) {
        const length = created.state.doc.length
        created.dispatch({
          selection: { anchor: Math.min(from, length), head: Math.min(to, length) },
          scrollIntoView: true,
        })
        created.focus()
      },
      complete() {
        created.focus()
        startCompletion(created)
      },
    })
    return () => {
      handleRef?.(null)
      view.current = null
      created.destroy()
    }
    // The editor is created once; later prop changes are dispatched below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const current = view.current
    if (!current) return
    const text = current.state.doc.toString()
    if (text === value) return
    current.dispatch({ changes: minimalChange(text, value), annotations: external.of(true) })
  }, [value])

  useEffect(() => {
    view.current?.dispatch({
      effects: [
        editable.current.reconfigure([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
        attributes.current.reconfigure(
          EditorView.contentAttributes.of({ "aria-label": label, ...(describedBy ? { "aria-describedby": describedBy } : {}) })
        ),
      ],
    })
  }, [readOnly, label, describedBy])

  useEffect(() => {
    const current = view.current
    // Problems belong to one exact text; never place them on another.
    if (!current || current.state.doc.toString() !== problemsText) return
    const length = current.state.doc.length
    const diagnostics: Diagnostic[] = problems.map((problem) => {
      const from = Math.min(problem.from, length)
      const to = Math.min(Math.max(problem.to, from === length ? from : from + 1), length)
      return {
        from,
        to,
        severity: "error",
        source: problem.source === "syntax" ? "JSONC" : problem.source === "schema" ? "Schema" : "检查",
        message: problem.message,
      }
    })
    current.dispatch(setDiagnostics(current.state, diagnostics))
  }, [problems, problemsText, value])

  return <div ref={host} className="h-full min-h-0" />
}
