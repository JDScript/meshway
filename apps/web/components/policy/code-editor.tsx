"use client"

import dynamic from "next/dynamic"

import { Skeleton } from "@/components/ui/skeleton"
import type { ReferenceSuggestions } from "@/lib/jsonc-complete"
import type { DocumentProblem } from "@/lib/policy-document"

export type CodeEditorHandle = {
  /** Selects [from, to) and focuses the editor. */
  focusAt: (from: number, to?: number) => void
  /** Opens schema completion at the cursor. */
  complete: () => void
}

export type CodeEditorProps = {
  value: string
  onChange: (text: string) => void
  /** Problems computed for `problemsText`; shown only while it is the buffer. */
  problems: readonly DocumentProblem[]
  problemsText: string
  /** JSON Schema document containing `schema` (for `$ref`). */
  schemaRoot: unknown
  schema: unknown
  references?: ReferenceSuggestions
  readOnly?: boolean
  label: string
  describedBy?: string
  handleRef?: (handle: CodeEditorHandle | null) => void
}

/**
 * JSONC policy editor (CodeMirror 6). Loaded on the client only: CodeMirror
 * needs the DOM, and keeping it out of the server bundle keeps the initial
 * page small. Everything is bundled locally; no workers or CDN.
 */
export const PolicyCodeEditor = dynamic<CodeEditorProps>(
  () => import("@/components/policy/code-editor-view").then((module) => module.CodeEditorView),
  {
    ssr: false,
    loading: () => <Skeleton className="h-full w-full rounded-none" />,
  }
)
