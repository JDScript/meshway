import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands"
import { json } from "@codemirror/lang-json"
import {
  bracketMatching,
  HighlightStyle,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language"
import { lintGutter, setDiagnostics, type Diagnostic } from "@codemirror/lint"
import {
  Annotation,
  Compartment,
  EditorState,
  RangeSetBuilder,
} from "@codemirror/state"
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

import { commentRanges } from "./jsonc"

export type EditorMark = {
  from: number
  to: number
  severity: "error" | "warning" | "info"
  message: string
}

export type EditorHandle = { focusAt: (from: number, to?: number) => void }

/** Marks a replacement made from outside (a graphical edit), which must not echo back. */
const external = Annotation.define<boolean>()

/** CodeMirror's own line inset, kept under the hanging indent. */
const LINE_INSET = "6px"
const hangs = new Map<number, Decoration>()
const hang = (columns: number) => {
  let decoration = hangs.get(columns)
  if (!decoration) {
    decoration = Decoration.line({
      attributes: {
        style: `padding-left: calc(${LINE_INSET} + ${columns}ch); text-indent: -${columns}ch`,
      },
    })
    hangs.set(columns, decoration)
  }
  return decoration
}

/** A wrapped line continues under its own indentation and one level more, not at the margin. */
const hangingIndent = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet
    constructor(view: EditorView) {
      this.decorations = this.build(view)
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged)
        this.decorations = this.build(update.view)
    }
    build(view: EditorView): DecorationSet {
      const builder = new RangeSetBuilder<Decoration>()
      for (const { from, to } of view.visibleRanges) {
        for (let at = from; at <= to;) {
          const line = view.state.doc.lineAt(at)
          const indent = /^[ \t]*/.exec(line.text)?.[0].length ?? 0
          builder.add(line.from, line.from, hang(indent + 2))
          at = line.to + 1
        }
      }
      return builder.finish()
    }
  },
  { decorations: (plugin) => plugin.decorations }
)

const theme = EditorView.theme({
  "&": {
    height: "100%",
    fontSize: "12.5px",
    color: "var(--foreground)",
    backgroundColor: "transparent",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--font-mono, ui-monospace), ui-monospace, monospace",
    lineHeight: "1.65",
  },
  ".cm-content": {
    caretColor: "var(--foreground)",
    padding: "12px 0 var(--commit-space, 76px)",
  },
  ".cm-line": { padding: "0 16px 0 12px" },
  ".cm-gutters": {
    backgroundColor: "transparent",
    color: "color-mix(in oklab, var(--muted-foreground) 70%, transparent)",
    border: "none",
    paddingLeft: "6px",
  },
  ".cm-lineNumbers .cm-gutterElement": { minWidth: "28px", padding: "0 4px" },
  ".cm-activeLine": { backgroundColor: "var(--fill-subtle)" },
  ".cm-activeLineGutter": {
    backgroundColor: "transparent",
    color: "var(--foreground)",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection":
    {
      backgroundColor: "color-mix(in oklab, var(--brand) 18%, transparent)",
    },
  "&.cm-focused .cm-matchingBracket": {
    backgroundColor: "var(--fill-selected)",
    outline: "none",
  },
  ".cm-cursor": { borderLeftColor: "var(--foreground)" },
  ".cm-tooltip": {
    backgroundColor: "var(--popover)",
    color: "var(--popover-foreground)",
    border: "1px solid var(--border)",
    borderRadius: "12px",
    overflow: "hidden",
  },
  ".cm-diagnostic": { padding: "6px 10px", fontFamily: "var(--font-sans)" },
  ".cm-diagnostic-error": { borderLeft: "3px solid var(--destructive)" },
  ".cm-diagnostic-warning": { borderLeft: "3px solid var(--warning)" },
  ".cm-diagnostic-info": { borderLeft: "3px solid var(--muted-foreground)" },
  ".cm-lintRange-error": {
    backgroundImage: "none",
    textDecoration: "underline wavy var(--destructive)",
    textUnderlineOffset: "3px",
  },
  ".cm-lintRange-warning": {
    backgroundImage: "none",
    textDecoration: "underline wavy var(--warning)",
    textUnderlineOffset: "3px",
  },
  ".cm-lintRange-info": {
    backgroundImage: "none",
    textDecoration: "underline dotted var(--muted-foreground)",
    textUnderlineOffset: "3px",
  },
  ".cm-gutter-lint": { width: "12px" },
  ".cm-jsonc-comment": {
    color: "var(--muted-foreground)",
    fontStyle: "italic",
  },
})

const highlight = HighlightStyle.define([
  { tag: tags.propertyName, color: "var(--foreground)", fontWeight: "500" },
  { tag: tags.string, color: "var(--chart-2)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--chart-3)" },
  {
    tag: [tags.punctuation, tags.separator, tags.squareBracket, tags.brace],
    color: "var(--muted-foreground)",
  },
])

/** The JSON grammar has no comments: they are marked from the JSONC scanner. */
const comments = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet
    constructor(view: EditorView) {
      this.decorations = scan(view)
    }
    update(update: ViewUpdate) {
      if (update.docChanged) this.decorations = scan(update.view)
    }
  },
  { decorations: (plugin) => plugin.decorations }
)
const commentMark = Decoration.mark({ class: "cm-jsonc-comment" })
function scan(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>()
  for (const range of commentRanges(view.state.doc.toString()))
    builder.add(range.from, range.to, commentMark)
  return builder.finish()
}

/** The smallest single change turning `from` into `to`, so the cursor stays put. */
function minimalChange(from: string, to: string) {
  let start = 0
  while (start < from.length && start < to.length && from[start] === to[start])
    start += 1
  let end = 0
  while (
    end < from.length - start &&
    end < to.length - start &&
    from[from.length - 1 - end] === to[to.length - 1 - end]
  )
    end += 1
  return {
    from: start,
    to: from.length - end,
    insert: to.slice(start, to.length - end),
  }
}

/**
 * The JSONC source editor. `value` is the draft: typing reports every
 * change, and a change made elsewhere (a graphical edit) is applied as the
 * smallest replacement. `marks` belong to `marksFor` exactly and are never
 * placed on other text.
 */
export function CodeEditor({
  value,
  onChange,
  marks,
  marksFor,
  readOnly,
  label,
  handle,
}: {
  value: string
  onChange: (value: string) => void
  marks: readonly EditorMark[]
  marksFor: string
  readOnly: boolean
  label: string
  handle?: (handle: EditorHandle | null) => void
}) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const latest = useRef(onChange)
  const editable = useRef(new Compartment())
  useEffect(() => {
    latest.current = onChange
  })

  useEffect(() => {
    const parent = host.current
    if (!parent) return
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
          json(),
          syntaxHighlighting(highlight),
          comments,
          lintGutter(),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          theme,
          EditorState.tabSize.of(2),
          EditorView.lineWrapping,
          hangingIndent,
          editable.current.of([
            EditorState.readOnly.of(readOnly),
            EditorView.editable.of(!readOnly),
          ]),
          EditorView.contentAttributes.of({
            "aria-label": label,
            autocapitalize: "off",
            autocorrect: "off",
            spellcheck: "false",
          }),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged) return
            if (update.transactions.some((tr) => tr.annotation(external)))
              return
            latest.current(update.state.doc.toString())
          }),
        ],
      }),
    })
    view.current = created
    handle?.({
      focusAt(from, to = from) {
        const length = created.state.doc.length
        created.dispatch({
          selection: {
            anchor: Math.min(from, length),
            head: Math.min(to, length),
          },
          scrollIntoView: true,
        })
        created.focus()
      },
    })
    return () => {
      handle?.(null)
      view.current = null
      created.destroy()
    }
    // Created once; later props are dispatched below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const current = view.current
    if (!current) return
    const text = current.state.doc.toString()
    if (text === value) return
    current.dispatch({
      changes: minimalChange(text, value),
      annotations: external.of(true),
    })
  }, [value])

  useEffect(() => {
    view.current?.dispatch({
      effects: editable.current.reconfigure([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
    })
  }, [readOnly])

  useEffect(() => {
    const current = view.current
    if (!current || current.state.doc.toString() !== marksFor) return
    const length = current.state.doc.length
    const diagnostics: Diagnostic[] = marks.map((mark) => {
      const from = Math.min(mark.from, length)
      const to = Math.min(
        Math.max(mark.to, from === length ? from : from + 1),
        length
      )
      return { from, to, severity: mark.severity, message: mark.message }
    })
    current.dispatch(setDiagnostics(current.state, diagnostics))
  }, [marks, marksFor, value])

  return <div ref={host} className="h-full min-h-0" />
}
