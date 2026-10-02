import { m } from "@/paraglide/messages.js"

import type { Problem, SchemaCode } from "./document"
import { lineColumn, type JsoncErrorCode } from "./jsonc"

const SYNTAX: Record<JsoncErrorCode, () => string> = {
  InvalidSymbol: () => m.policy_syntax_symbol(),
  InvalidNumberFormat: () => m.policy_syntax_number(),
  PropertyNameExpected: () => m.policy_syntax_property(),
  ValueExpected: () => m.policy_syntax_value(),
  ColonExpected: () => m.policy_syntax_colon(),
  CommaExpected: () => m.policy_syntax_comma(),
  CloseBraceExpected: () => m.policy_syntax_brace(),
  CloseBracketExpected: () => m.policy_syntax_bracket(),
  EndOfFileExpected: () => m.policy_syntax_end(),
  InvalidCommentToken: () => m.policy_syntax_comment(),
  UnexpectedEndOfComment: () => m.policy_syntax_comment_end(),
  UnexpectedEndOfString: () => m.policy_syntax_string_end(),
  UnexpectedEndOfNumber: () => m.policy_syntax_number(),
  InvalidUnicode: () => m.policy_syntax_unicode(),
  InvalidEscapeCharacter: () => m.policy_syntax_escape(),
  InvalidCharacter: () => m.policy_syntax_character(),
  DuplicateKey: () => m.policy_syntax_duplicate_key(),
  Empty: () => m.policy_syntax_empty(),
  "<unknown ParseErrorCode>": () => m.policy_syntax_other(),
}

const SCHEMA: Record<
  Exclude<SchemaCode, "unknown_key" | "missing">,
  () => string
> = {
  type: () => m.policy_schema_type(),
  value: () => m.policy_schema_value(),
  union: () => m.policy_schema_union(),
  other: () => m.policy_schema_other(),
}

/** What is wrong, without where. */
export function problemReason(problem: Problem): string {
  if (problem.kind === "syntax") return SYNTAX[problem.code]()
  if (problem.kind === "id")
    return problem.code === "duplicate"
      ? m.policy_id_duplicate()
      : m.policy_id_syntax()
  if (problem.code === "unknown_key")
    return m.policy_schema_unknown_key({ key: problem.key ?? "" })
  if (problem.code === "missing")
    return m.policy_schema_missing({
      field: String(problem.path[problem.path.length - 1] ?? ""),
    })
  return SCHEMA[problem.code]()
}

/** "Line 3 · Comma expected". */
export function problemMessage(problem: Problem, text: string): string {
  return m.policy_problem_at({
    line: lineColumn(text, problem.from).line,
    reason: problemReason(problem),
  })
}
