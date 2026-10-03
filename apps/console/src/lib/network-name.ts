import type { NetworkNameRules } from "@meshway/control-client"

// A network's name, checked as the controller checks it, by the rules it
// publishes (`getAuthOptions().networkNameRules`): the raw input's UTF-8
// bytes and Unicode Cc characters first, then the Unicode White_Space at
// its edges is trimmed and the rest counted in code points. JavaScript's
// `trim()` and `String.length` differ from both and are not used.

/** Unicode White_Space, as the rules name it; U+FEFF is not in it. */
const WHITE_SPACE = new Set([
  0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x85, 0xa0, 0x1680, 0x2000, 0x2001,
  0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a,
  0x2028, 0x2029, 0x202f, 0x205f, 0x3000,
])

const CONTROL = /\p{Cc}/u

const isWhiteSpace = (char: string) =>
  WHITE_SPACE.has(char.codePointAt(0) ?? -1)

/** `value` without the Unicode White_Space at its edges. */
export function trimWhiteSpace(value: string): string {
  const chars = [...value]
  let start = 0
  let end = chars.length
  while (start < end && isWhiteSpace(chars[start] ?? "")) start++
  while (end > start && isWhiteSpace(chars[end - 1] ?? "")) end--
  return chars.slice(start, end).join("")
}

export type NetworkNameProblem = "bytes" | "control" | "empty" | "length"

export type NetworkNameCheck =
  { ok: true; name: string } | { ok: false; problem: NetworkNameProblem }

/** The name to send for what was typed, or why the controller would refuse it. */
export function checkNetworkName(
  raw: string,
  rules: NetworkNameRules
): NetworkNameCheck {
  if (new TextEncoder().encode(raw).length > rules.maxInputUtf8Bytes)
    return { ok: false, problem: "bytes" }
  if (rules.forbidControlCharacters && CONTROL.test(raw))
    return { ok: false, problem: "control" }
  const name = trimWhiteSpace(raw)
  const length = [...name].length
  if (length === 0 && rules.minCodepoints > 0)
    return { ok: false, problem: "empty" }
  if (length < rules.minCodepoints || length > rules.maxCodepoints)
    return { ok: false, problem: "length" }
  return { ok: true, name }
}
