import type {
  RuleSet,
  RuleSetFormat,
  RuleSetView,
} from "@meshway/control-client"

import { pointerOf } from "./jsonc"
import {
  badDestination,
  EXACT_PREFIX,
  matchKind,
  SUBDOMAIN_PREFIX,
} from "./routing"

// Rule sets as the form edits them (rule-sets.md): inline prefixes, domain
// suffixes and exact names (written as a route's match is), or a remote HTTPS source the controller downloads and
// converts (when a draft is checked and applied, on its interval, and when
// refreshed). Where a set's content stands.

export type RuleSetMap = Record<string, RuleSet>

export const RULE_SET_FORMATS: readonly RuleSetFormat[] = [
  "sing-box-source",
  "mihomo-domain",
  "mihomo-ipcidr",
  "mihomo-domain-text",
  "mihomo-ipcidr-text",
  "classical",
]

/**
 * The catalogs the controller has built in (rule-sets.md, `ruleSetProviders`):
 * `geosite:<name>` and `geoip:<code>` read from meshway-org/meshway-rules
 * unless the configuration replaces or turns one off.
 */
export const BUILTIN_PROVIDERS: readonly string[] = ["geosite", "geoip"]

/** The providers a reference can name: the built-in ones not turned off, and the configuration's own. */
export function effectiveProviders(
  overrides: Readonly<Record<string, unknown>>
): string[] {
  return [
    ...BUILTIN_PROVIDERS.filter((name) => overrides[name] !== null),
    ...Object.entries(overrides)
      .filter(
        ([name, provider]) =>
          provider !== null && !BUILTIN_PROVIDERS.includes(name)
      )
      .map(([name]) => name)
      .sort(),
  ]
}

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
function durationMs(value: string): number | null {
  const found = /^([1-9][0-9]*)(ms|s|m|h)$/.exec(value)
  const factor = found?.[2] === undefined ? null : unitMs(found[2])
  if (!found?.[1] || factor === null) return null
  const ms = Number(found[1]) * factor
  return Number.isSafeInteger(ms) ? ms : null
}

/** An HTTPS source as the controller takes it: a host, no credentials, no fragment, no spaces. */
function isSourceUrl(value: string): boolean {
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
        domain: sorted(set.domain),
        domainSubdomain: sorted(set.domainSubdomain),
        ipCIDR: sorted(set.ipCIDR),
      }
    : {
        type: "remote",
        url: set.url,
        format: set.format,
        updateInterval: durationMs(set.updateInterval ?? DEFAULT_INTERVAL),
      }
}

const sameRuleSet = (a: RuleSet, b: RuleSet) =>
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

type Inline = Extract<RuleSet, { type: "inline" }>

/** An inline set's values as chips: domain suffixes, exact names, names below (`.`), then prefixes. */
export const inlineValues = (set: Inline) => [
  ...(set.domainSuffix ?? []),
  ...(set.domain ?? []).map((name) => EXACT_PREFIX + name),
  ...(set.domainSubdomain ?? []).map((name) => SUBDOMAIN_PREFIX + name),
  ...(set.ipCIDR ?? []),
]

/** The arrays `inlineValues` joins, in its order, with their lengths. */
export const inlineParts = (set: Inline) =>
  [
    ["domainSuffix", set.domainSuffix?.length ?? 0],
    ["domain", set.domain?.length ?? 0],
    ["domainSubdomain", set.domainSubdomain?.length ?? 0],
    ["ipCIDR", set.ipCIDR?.length ?? 0],
  ] as const

/** Chips as an inline set; a value of no kind is kept as a domain suffix, for the controller to refuse. */
export function inlineOf(values: readonly string[]): RuleSet {
  const domainSuffix: string[] = []
  const domain: string[] = []
  const domainSubdomain: string[] = []
  const ipCIDR: string[] = []
  for (const value of values) {
    const kind = matchKind(value)
    if (kind === "exact") domain.push(value.slice(EXACT_PREFIX.length))
    else if (kind === "subdomain")
      domainSubdomain.push(value.slice(SUBDOMAIN_PREFIX.length))
    else if (kind === "cidr") ipCIDR.push(value)
    else domainSuffix.push(value)
  }
  const out: RuleSet = { type: "inline" }
  if (domainSuffix.length > 0) out.domainSuffix = domainSuffix
  if (domain.length > 0) out.domain = domain
  if (domainSubdomain.length > 0) out.domainSubdomain = domainSubdomain
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

/** A key a set may have: a name, no white space (a set is named in a match as `set:<key>`), no `:` (it names a catalog set). */
const isSetKey = (key: string) => key !== "" && !/[\s:]/.test(key)

export type RuleSetCode =
  "empty" | "destination" | "url" | "duration" | "set_key"

export type RuleSetProblem = { path: string; code: RuleSetCode }

export function ruleSetProblems(sets: RuleSetMap): RuleSetProblem[] {
  const out: RuleSetProblem[] = []
  for (const [key, set] of Object.entries(sets)) {
    const at = (field?: string) =>
      pointerOf(field ? ["ruleSets", key, field] : ["ruleSets", key])
    if (!isSetKey(key)) out.push({ path: at(), code: "set_key" })
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

/** Whether two sets download the same content: the same remote URL and format (the interval aside), or the same inline set. */
function sameSource(a: RuleSet, b: RuleSet): boolean {
  if (a.type === "remote" && b.type === "remote")
    return a.url === b.url && a.format === b.format
  return sameRuleSet(a, b)
}

/**
 * Where a set's content stands (rule-sets.md, "新 Controller 候选用法").
 * The controller downloads and converts a remote source that is new or
 * replaced when the draft is checked (tentatively, saving nothing) and again
 * when it is applied, so one apply may add a set and reference it. Its
 * status is for the definition at the current revision; a source only the
 * draft has has none until applied.
 *
 * - `inline`: its content is the configuration itself.
 * - `new`: a remote set only this draft has: prepared when checked and applied.
 * - `replaced`: the draft changes the source (URL or format): prepared again; the status is the saved source's.
 * - `unread`: no status read (yet, or not readable).
 * - `ready`: content selected for the current revision.
 * - `failing`: the last check failed; content from an earlier success is still selected.
 * - `missing`: no content, never an empty set: refreshing, or applying the draft, downloads it again.
 */
export type Preparation =
  "inline" | "new" | "replaced" | "unread" | "ready" | "failing" | "missing"

export function preparationOf(
  draft: RuleSet,
  saved: RuleSet | undefined,
  view: RuleSetView | undefined
): Preparation {
  if (draft.type === "inline") return "inline"
  if (saved === undefined) return "new"
  if (!sameSource(draft, saved)) return "replaced"
  if (view === undefined) return "unread"
  if (view.artifact) return view.problem ? "failing" : "ready"
  return "missing"
}

/** Whether the controller can refresh this set now: its source is saved at the current revision as the draft has it. */
export const refreshable = (preparation: Preparation) =>
  preparation === "ready" ||
  preparation === "failing" ||
  preparation === "missing"
