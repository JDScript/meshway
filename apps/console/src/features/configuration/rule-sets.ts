import type {
  RuleSet,
  RuleSetFormat,
  RuleSetView,
} from "@meshway/control-client"

import { pointerOf } from "./jsonc.ts"
import { badDestination, matchKind } from "./routing.ts"

// Rule sets as the form edits them (rule-sets.md): inline prefixes and
// domain suffixes, or a remote HTTPS source the controller downloads and
// converts when asked (refresh). What a set's status means for using it.

export type RuleSetMap = Record<string, RuleSet>

export const RULE_SET_FORMATS: readonly RuleSetFormat[] = [
  "sing-box-source",
  "mihomo-domain",
  "mihomo-ipcidr",
]

/** The update interval the controller keeps when none is written. */
export const DEFAULT_INTERVAL = "24h"

const unitMs = (unit: string): number | null =>
  unit === "ms"
    ? 1
    : unit === "s"
      ? 1_000
      : unit === "m"
        ? 60_000
        : unit === "h"
          ? 3_600_000
          : null

/** A positive duration as the controller takes it (`500ms`, `30s`, `15m`, `24h`), in milliseconds; null when it is none. */
export function durationMs(value: string): number | null {
  const found = /^([1-9][0-9]*)(ms|s|m|h)$/.exec(value)
  const factor = found?.[2] === undefined ? null : unitMs(found[2])
  if (!found?.[1] || factor === null) return null
  const ms = Number(found[1]) * factor
  return Number.isSafeInteger(ms) ? ms : null
}

/** An HTTPS source as the controller takes it: a host, no credentials, no fragment, no spaces. */
export function isSourceUrl(value: string): boolean {
  if (!value.startsWith("https://") || value.length > 2048) return false
  if (/[\s#]/.test(value) || [...value].some((c) => c < " ")) return false
  const authority = value.slice(8).split(/[/?]/)[0] ?? ""
  if (authority === "" || authority.includes("@")) return false
  try {
    return new URL(value).hostname !== ""
  } catch {
    return false
  }
}

const sorted = (values: readonly string[] | undefined) =>
  [...new Set(values ?? [])].sort()

function comparable(set: RuleSet) {
  return set.type === "inline"
    ? {
        type: "inline",
        domainSuffix: sorted(set.domainSuffix),
        ipCIDR: sorted(set.ipCIDR),
      }
    : {
        type: "remote",
        url: set.url,
        format: set.format,
        updateInterval: durationMs(set.updateInterval ?? DEFAULT_INTERVAL),
      }
}

export const sameRuleSet = (a: RuleSet, b: RuleSet) =>
  JSON.stringify(comparable(a)) === JSON.stringify(comparable(b))

export function sameRuleSets(a: RuleSetMap, b: RuleSetMap): boolean {
  const keys = Object.keys(a)
  if (keys.length !== Object.keys(b).length) return false
  return keys.every((key) => {
    const x = a[key]
    const y = b[key]
    return x !== undefined && y !== undefined && sameRuleSet(x, y)
  })
}

/** An inline set's values as chips: domain suffixes, then prefixes. */
export const inlineValues = (set: Extract<RuleSet, { type: "inline" }>) => [
  ...(set.domainSuffix ?? []),
  ...(set.ipCIDR ?? []),
]

/** Chips as an inline set; a value of no kind is kept as a domain suffix, for the controller to refuse. */
export function inlineOf(values: readonly string[]): RuleSet {
  const domainSuffix = values.filter((value) => matchKind(value) !== "cidr")
  const ipCIDR = values.filter((value) => matchKind(value) === "cidr")
  const out: RuleSet = { type: "inline" }
  if (domainSuffix.length > 0) out.domainSuffix = domainSuffix
  if (ipCIDR.length > 0) out.ipCIDR = ipCIDR
  return out
}

/** A set as the form writes it: the interval only when it is not the default. */
export function writtenRuleSet(set: RuleSet): RuleSet {
  if (set.type === "inline") return inlineOf(inlineValues(set))
  const out: RuleSet = { type: "remote", url: set.url, format: set.format }
  if (
    set.updateInterval !== undefined &&
    durationMs(set.updateInterval) !== durationMs(DEFAULT_INTERVAL)
  )
    out.updateInterval = set.updateInterval
  return out
}

/** A key a set may have: a name, no white space (a set is named in a match as `set:<key>`). */
export const isSetKey = (key: string) => key !== "" && !/\s/.test(key)

export type RuleSetCode = "empty" | "destination" | "url" | "duration" | "key"

export type RuleSetProblem = { path: string; code: RuleSetCode }

export function ruleSetProblems(sets: RuleSetMap): RuleSetProblem[] {
  const out: RuleSetProblem[] = []
  for (const [key, set] of Object.entries(sets)) {
    const at = (field?: string) =>
      pointerOf(field ? ["ruleSets", key, field] : ["ruleSets", key])
    if (!isSetKey(key)) out.push({ path: at(), code: "key" })
    if (set.type === "inline") {
      const values = inlineValues(set)
      if (values.length === 0) out.push({ path: at(), code: "empty" })
      else if (
        values.some(
          (value) => matchKind(value) === "set" || badDestination(value)
        )
      )
        out.push({ path: at(), code: "destination" })
    } else {
      if (!isSourceUrl(set.url)) out.push({ path: at("url"), code: "url" })
      if (
        set.updateInterval !== undefined &&
        durationMs(set.updateInterval) === null
      )
        out.push({ path: at("updateInterval"), code: "duration" })
    }
  }
  return out
}

/**
 * Where a set stands for being used in routing (rule-sets.md, "新 Controller
 * 候选用法"). The controller's status is for the definition at the current
 * revision; a draft's own definition has none until applied.
 *
 * - `inline`: its content is the configuration itself.
 * - `unsaved`: a remote set only this draft has: apply it unreferenced, then refresh.
 * - `changed`: a remote set the draft changes: the status is the saved definition's; apply, then refresh.
 * - `unread`: no status read (yet, or not readable).
 * - `ready`: content selected for the current revision.
 * - `failing`: the last check failed; content from an earlier success is still selected.
 * - `missing`: no content: a route naming it cannot be applied until a refresh succeeds.
 */
export type Preparation =
  "inline" | "unsaved" | "changed" | "unread" | "ready" | "failing" | "missing"

export function preparationOf(
  draft: RuleSet,
  saved: RuleSet | undefined,
  view: RuleSetView | undefined
): Preparation {
  if (draft.type === "inline") return "inline"
  if (saved === undefined) return "unsaved"
  if (!sameRuleSet(draft, saved)) return "changed"
  if (view === undefined) return "unread"
  if (view.artifact) return view.problem ? "failing" : "ready"
  return "missing"
}

/** Whether a set can be referenced and applied as it is now. */
export const usable = (preparation: Preparation) =>
  preparation === "inline" ||
  preparation === "ready" ||
  preparation === "failing"

/** Whether the controller can refresh this set now: it is saved at the current revision as the draft has it. */
export const refreshable = (preparation: Preparation) =>
  preparation === "ready" ||
  preparation === "failing" ||
  preparation === "missing"
