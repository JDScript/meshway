import type { ConfigurationIssue } from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

import type { Checked } from "./draft-state"
import {
  findKey,
  parseJsonc,
  pointerOf,
  spanOfPointer,
  type JsoncErrorCode,
  type JsoncNode,
  type Span,
} from "./jsonc"
import { requirementText } from "./preparation"
import { checkSchema, type SchemaIssue } from "./schema-check"

// What is wrong with the draft's text, placed on it: JSONC syntax (found
// here, and it stops applying), hints from the controller's schema (found
// here; the controller decides), and the controller's own diagnostics from
// its last check, by JSON Pointer.

export type SourceProblem = Span & {
  origin: "syntax" | "schema" | "controller"
  /** The JSON Pointer it is about, when it has one. */
  path: string | null
  message: string
}

const SYNTAX: Record<JsoncErrorCode, () => string> = {
  InvalidSymbol: () => m.configuration_syntax_symbol(),
  InvalidNumberFormat: () => m.configuration_syntax_number(),
  PropertyNameExpected: () => m.configuration_syntax_property(),
  ValueExpected: () => m.configuration_syntax_value(),
  ColonExpected: () => m.configuration_syntax_colon(),
  CommaExpected: () => m.configuration_syntax_comma(),
  CloseBraceExpected: () => m.configuration_syntax_brace(),
  CloseBracketExpected: () => m.configuration_syntax_bracket(),
  EndOfFileExpected: () => m.configuration_syntax_end(),
  InvalidCommentToken: () => m.configuration_syntax_comment(),
  UnexpectedEndOfComment: () => m.configuration_syntax_comment_end(),
  UnexpectedEndOfString: () => m.configuration_syntax_string_end(),
  UnexpectedEndOfNumber: () => m.configuration_syntax_number(),
  InvalidUnicode: () => m.configuration_syntax_unicode(),
  InvalidEscapeCharacter: () => m.configuration_syntax_escape(),
  InvalidCharacter: () => m.configuration_syntax_character(),
  DuplicateKey: () => m.configuration_syntax_duplicate_key(),
  Empty: () => m.configuration_syntax_empty(),
  "<unknown ParseErrorCode>": () => m.configuration_syntax_other(),
}

function schemaMessage(issue: SchemaIssue): string {
  switch (issue.code) {
    case "type":
      return m.configuration_schema_type({ expected: issue.detail })
    case "required":
      return m.configuration_schema_required({ key: issue.detail })
    case "unknown_key":
      return m.configuration_schema_unknown_key({ key: issue.detail })
    case "enum":
      return m.configuration_schema_enum({ allowed: issue.detail })
    case "variant":
      return issue.detail
        ? m.configuration_schema_variant({ allowed: issue.detail })
        : m.configuration_schema_variant_any()
    case "range":
      return m.configuration_schema_range({ limit: issue.detail })
    case "length":
      return m.configuration_schema_length({ limit: issue.detail })
    case "pattern":
      return m.configuration_schema_pattern()
  }
}

/** Where a pointer's problem goes: on its value, the deepest value holding it, or the start. */
function placeAt(tree: JsoncNode | null, text: string, pointer: string): Span {
  const start = { from: 0, to: Math.min(1, text.length) }
  if (!tree || pointer === "") return start
  const span = spanOfPointer(tree, pointer)
  return span ?? start
}

export type SourceReading = {
  /** The text parses as JSONC; it may still be refused. */
  valid: boolean
  syntax: SourceProblem[]
  schema: SourceProblem[]
  controller: SourceProblem[]
  /** The controller's diagnostics are about an earlier text. */
  stale: boolean
}

export function readSource(
  text: string,
  schema: Readonly<Record<string, unknown>> | null,
  checked: Checked | null
): SourceReading {
  const parsed = parseJsonc(text)
  const tree = parsed.ok ? parsed.tree : null
  const syntax: SourceProblem[] = parsed.ok
    ? []
    : parsed.errors.map((error) => ({
        origin: "syntax",
        path: null,
        from: error.offset,
        to: error.offset + error.length,
        message: SYNTAX[error.code](),
      }))
  const hints: SourceProblem[] =
    parsed.ok && schema
      ? checkSchema(schema, parsed.value).map((issue) => {
          const path = pointerOf(issue.path)
          // A key's own problem is marked on the key (the last segment).
          const keyNode =
            issue.onKey && tree
              ? findKey(
                  tree,
                  issue.path.slice(0, -1),
                  String(issue.path[issue.path.length - 1])
                )
              : undefined
          const span = keyNode
            ? { from: keyNode.offset, to: keyNode.offset + keyNode.length }
            : placeAt(tree, text, path)
          return {
            origin: "schema",
            path,
            ...span,
            message: schemaMessage(issue),
          }
        })
      : []
  // Only a check of a document is about a text.
  const document = checked?.input.kind === "document" ? checked : null
  const checkedText =
    document?.input.kind === "document" ? document.input.text : null
  const controller: SourceProblem[] = document
    ? [
        ...document.issues.map((issue: ConfigurationIssue) => ({
          origin: "controller" as const,
          path: issue.path,
          ...placeAt(tree, text, issue.path),
          message: issue.message,
        })),
        ...document.requirements.map((requirement) => ({
          origin: "controller" as const,
          path: requirement.path,
          ...placeAt(tree, text, requirement.path),
          message: requirementText(requirement),
        })),
      ]
    : []
  const byPlace = (a: SourceProblem, b: SourceProblem) => a.from - b.from
  return {
    valid: parsed.ok,
    syntax: syntax.sort(byPlace),
    schema: hints.sort(byPlace),
    controller: controller.sort(byPlace),
    stale: checkedText !== null && checkedText !== text,
  }
}
