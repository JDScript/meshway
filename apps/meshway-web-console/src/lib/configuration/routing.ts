import type {
  EgressRule,
  ProfileAssignment,
  RouteAction,
  RouteMatch,
  RoutingConfiguration,
  RoutingProfile,
  RuleAction,
} from "@meshway/control-client"

import { isIpv4 } from "@/lib/ip"

import { pointerOf, type JsonPath } from "./jsonc"
import { cidrFamily, isDomain, selectorKind } from "./selectors"

// The routing section as the form edits it (routing.md): Mesh routes and
// egress rules are ordered first-match lists; profiles are a named map;
// assignments add up. Values are compared the way the controller reads
// them: omitted lists are empty, an omitted remark is "", `locked` is
// false and an exit's `via` is []; a match's and an assignment's sets mean
// the same in any order, while exits and via nodes are in priority order.

const NODE =
  /^node:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

/** `node:<id>`: via nodes and exits are concrete nodes only. */
export const isNodeSelector = (value: string) => NODE.test(value)

/** Moves the item at `index` by `delta` places; the list as it was when it cannot move. */
export function moved<T>(
  items: readonly T[],
  index: number,
  delta: number
): T[] {
  const to = index + delta
  if (index < 0 || index >= items.length || to < 0 || to >= items.length)
    return [...items]
  const out = [...items]
  const [item] = out.splice(index, 1)
  if (item !== undefined) out.splice(to, 0, item)
  return out
}

// A match's destinations are edited as one set of chips: a domain suffix
// as typed (the name and every name under it), an exact name after `=`
// (that name alone), a CIDR prefix, or a rule set as `set:<key>`.

export const SET_PREFIX = "set:"
export const EXACT_PREFIX = "="

export type MatchKind = "domain" | "exact" | "cidr" | "set"

export function matchKind(value: string): MatchKind | null {
  if (value.startsWith(SET_PREFIX))
    return value.length > SET_PREFIX.length ? "set" : null
  if (value.startsWith(EXACT_PREFIX))
    return isDomain(value.slice(EXACT_PREFIX.length)) ? "exact" : null
  if (cidrFamily(value) !== null) return "cidr"
  return isDomain(value) ? "domain" : null
}

/** A match as chips: domain suffixes, exact names, prefixes, then rule sets. */
export const matchValues = (match: RouteMatch): string[] => [
  ...(match.domainSuffix ?? []),
  ...(match.domain ?? []).map((name) => EXACT_PREFIX + name),
  ...(match.ipCIDR ?? []),
  ...(match.ruleSets ?? []).map((key) => SET_PREFIX + key),
]

/** The arrays `matchValues` joins, in its order, with their lengths: where a pointer into the match is among the chips. */
export const matchParts = (match: RouteMatch) =>
  [
    ["domainSuffix", match.domainSuffix?.length ?? 0],
    ["domain", match.domain?.length ?? 0],
    ["ipCIDR", match.ipCIDR?.length ?? 0],
    ["ruleSets", match.ruleSets?.length ?? 0],
  ] as const

/** Chips as a match; a value of no kind is kept as a domain suffix, for the controller to refuse. */
export function matchOf(values: readonly string[]): RouteMatch {
  const domainSuffix: string[] = []
  const domain: string[] = []
  const ipCIDR: string[] = []
  const ruleSets: string[] = []
  for (const value of values) {
    const kind = matchKind(value)
    if (kind === "set") ruleSets.push(value.slice(SET_PREFIX.length))
    else if (kind === "exact") domain.push(value.slice(EXACT_PREFIX.length))
    else if (kind === "cidr") ipCIDR.push(value)
    else domainSuffix.push(value)
  }
  const out: RouteMatch = {}
  if (domainSuffix.length > 0) out.domainSuffix = domainSuffix
  if (domain.length > 0) out.domain = domain
  if (ipCIDR.length > 0) out.ipCIDR = ipCIDR
  if (ruleSets.length > 0) out.ruleSets = ruleSets
  return out
}

/** The rule sets a routing configuration names, each once. */
export function referencedSets(routing: RoutingConfiguration): string[] {
  const egress = routing.egress ?? {}
  const rules = [
    ...(egress.rules ?? []),
    ...Object.values(egress.profiles ?? {}).flatMap(
      (profile) => profile.rules ?? []
    ),
  ]
  return [...new Set(rules.flatMap((rule) => rule.match.ruleSets ?? []))]
}

/** Every reference to rule set `from` renamed to `to`. */
export function renameSetReferences(
  routing: RoutingConfiguration,
  from: string,
  to: string
): RoutingConfiguration {
  const rename = (rule: EgressRule): EgressRule =>
    rule.match.ruleSets?.includes(from)
      ? {
          ...rule,
          match: {
            ...rule.match,
            ruleSets: rule.match.ruleSets.map((key) =>
              key === from ? to : key
            ),
          },
        }
      : rule
  const egress = routing.egress
  if (!egress) return routing
  return {
    ...routing,
    egress: {
      ...egress,
      ...(egress.rules ? { rules: egress.rules.map(rename) } : {}),
      ...(egress.profiles
        ? {
            profiles: Object.fromEntries(
              Object.entries(egress.profiles).map(([key, profile]) => [
                key,
                profile.rules
                  ? { ...profile, rules: profile.rules.map(rename) }
                  : profile,
              ])
            ),
          }
        : {}),
    },
  }
}

/** Every reference to profile `from` (assignments, the automatic profile) renamed to `to`. */
export function renameProfile(
  routing: RoutingConfiguration,
  from: string,
  to: string
): RoutingConfiguration {
  const egress = routing.egress ?? {}
  const swap = (value: string) => (value === from ? to : value)
  const profiles = Object.fromEntries(
    Object.entries(egress.profiles ?? {}).map(([key, profile]) => [
      key === from ? to : key,
      profile,
    ])
  )
  return {
    ...routing,
    egress: {
      ...egress,
      profiles,
      ...(egress.assignments
        ? {
            assignments: egress.assignments.map((assignment) => ({
              ...assignment,
              allowed: assignment.allowed.map(swap),
              default: swap(assignment.default),
            })),
          }
        : {}),
      ...(egress.auto
        ? {
            auto: {
              fallback: swap(egress.auto.fallback),
              rules: egress.auto.rules.map((rule) => ({
                ...rule,
                profile: swap(rule.profile),
              })),
            },
          }
        : {}),
    },
  }
}

export type ProfileKind = "mesh" | "auto" | "global" | "custom"

/** The kind of a profile name: the built-ins (`mesh`, `auto`, `global:<node>`) or one of the configuration's. */
export function profileKind(name: string): ProfileKind {
  if (name === "mesh") return "mesh"
  if (name === "auto") return "auto"
  if (name.startsWith("global:")) return "global"
  return "custom"
}

/** A profile name the configuration may define: not a built-in's, not empty, no white space. */
const isProfileName = (name: string) =>
  name !== "" &&
  !/\s/.test(name) &&
  name !== "mesh" &&
  name !== "auto" &&
  !name.startsWith("global:")

export type ProfileChoice = { value: string; kind: ProfileKind; node?: string }

/**
 * The profiles an assignment or the automatic profile can name: Mesh only,
 * automatic (where the configuration has it and it may be named), one
 * global profile per node enabled as an exit, then the configuration's.
 */
export function profileChoices(
  routing: RoutingConfiguration,
  exits: readonly string[],
  withAuto: boolean
): ProfileChoice[] {
  const out: ProfileChoice[] = [{ value: "mesh", kind: "mesh" }]
  if (withAuto && routing.egress?.auto)
    out.push({ value: "auto", kind: "auto" })
  for (const id of exits)
    out.push({ value: `global:${id}`, kind: "global", node: id })
  for (const key of Object.keys(routing.egress?.profiles ?? {}))
    out.push({ value: key, kind: "custom" })
  return out
}

/** A country or region as the automatic profile takes it: two uppercase letters. */
export const isCountry = (value: string) => /^[A-Z]{2}$/.test(value)

// Comparing: the value the controller reads, with sets sorted.

const sorted = (values: readonly string[] | undefined) =>
  [...new Set(values ?? [])].sort()

const comparableAction = (action: RouteAction) =>
  action.type === "exit"
    ? { type: "exit", nodes: action.nodes, via: action.via ?? [] }
    : { type: action.type }

const comparableRule = (rule: EgressRule) => ({
  remark: rule.remark ?? "",
  match: {
    domainSuffix: sorted(rule.match.domainSuffix),
    domain: sorted(rule.match.domain),
    ipCIDR: sorted(rule.match.ipCIDR),
    ruleSets: sorted(rule.match.ruleSets),
  },
  action: comparableAction(rule.action),
})

const comparableProfile = (profile: RoutingProfile) => ({
  rules: (profile.rules ?? []).map(comparableRule),
  fallback: comparableAction(profile.fallback),
})

const comparableAssignment = (assignment: ProfileAssignment) => ({
  target: sorted(assignment.target),
  allowed: sorted(assignment.allowed),
  default: assignment.default,
  locked: assignment.locked ?? false,
})

function comparable(routing: RoutingConfiguration) {
  const egress = routing.egress ?? {}
  const profiles = egress.profiles ?? {}
  return {
    mesh: (routing.mesh ?? []).map((rule) => ({
      remark: rule.remark ?? "",
      src: sorted(rule.src),
      dst: sorted(rule.dst),
      via: rule.via,
    })),
    rules: (egress.rules ?? []).map(comparableRule),
    profiles: Object.keys(profiles)
      .sort()
      .map((key) => {
        const profile = profiles[key]
        return [key, profile ? comparableProfile(profile) : null]
      }),
    assignments: (egress.assignments ?? []).map(comparableAssignment),
    auto: egress.auto
      ? {
          fallback: egress.auto.fallback,
          rules: egress.auto.rules.map((rule) => ({
            countries: sorted(rule.countries),
            profile: rule.profile,
          })),
        }
      : null,
  }
}

export const sameRouting = (
  a: RoutingConfiguration,
  b: RoutingConfiguration
): boolean => JSON.stringify(comparable(a)) === JSON.stringify(comparable(b))

// Writing: what the form sends, with the defaults left out.

type ExitAction = Extract<RouteAction, { type: "exit" }>

const writtenExit = (action: ExitAction): ExitAction =>
  action.via && action.via.length > 0
    ? { type: "exit", nodes: action.nodes, via: action.via }
    : { type: "exit", nodes: action.nodes }

const writtenRuleAction = (action: RuleAction): RuleAction =>
  action.type === "exit" ? writtenExit(action) : { type: action.type }

const writtenFallback = (action: RouteAction): RouteAction =>
  action.type === "exit" ? writtenExit(action) : { type: action.type }

const writtenRule = (rule: EgressRule): EgressRule => {
  const out: EgressRule = {
    match: matchOf(matchValues(rule.match)),
    action: writtenRuleAction(rule.action),
  }
  if (rule.remark) out.remark = rule.remark
  return out
}

/** The routing as the form writes it. */
export function writtenRouting(
  routing: RoutingConfiguration
): RoutingConfiguration {
  const egress = routing.egress ?? {}
  const out: RoutingConfiguration = {
    mesh: (routing.mesh ?? []).map((rule) =>
      rule.remark
        ? { remark: rule.remark, src: rule.src, dst: rule.dst, via: rule.via }
        : { src: rule.src, dst: rule.dst, via: rule.via }
    ),
    egress: {
      rules: (egress.rules ?? []).map(writtenRule),
      profiles: Object.fromEntries(
        Object.entries(egress.profiles ?? {}).map(([key, profile]) => [
          key,
          {
            rules: (profile.rules ?? []).map(writtenRule),
            fallback: writtenFallback(profile.fallback),
          },
        ])
      ),
      assignments: (egress.assignments ?? []).map((assignment) =>
        assignment.locked
          ? { ...assignment, locked: true }
          : {
              target: assignment.target,
              allowed: assignment.allowed,
              default: assignment.default,
            }
      ),
    },
  }
  if (egress.auto && out.egress) out.egress.auto = egress.auto
  return out
}

// What the routing cannot be saved with, found before asking the
// controller, at the pointers it uses (or the field under them).

export type RoutingCode =
  | "empty"
  | "selector"
  | "node"
  | "match"
  | "destination"
  | "exits"
  | "country"
  | "profileName"

export type RoutingProblem = { path: string; code: RoutingCode }

/** An IPv4 prefix written with host bits set (`10.0.0.1/8`): the controller takes only the network. */
function hostBitsSet(cidr: string): boolean {
  const [address, length] = cidr.split("/")
  if (address === undefined || length === undefined || !isIpv4(address))
    return false
  const bits = Number(length)
  const value = address
    .split(".")
    .reduce((sum, part) => sum * 256 + Number(part), 0)
  const hostSize = 2 ** (32 - bits)
  return value % hostSize !== 0
}

/** A match value the controller will not take: no kind, or a prefix with host bits. */
export const badDestination = (value: string) => {
  const kind = matchKind(value)
  return kind === null || (kind === "cidr" && hostBitsSet(value))
}

function actionProblems(
  out: RoutingProblem[],
  path: JsonPath,
  action: RouteAction
) {
  if (action.type !== "exit") return
  if (action.nodes.length === 0)
    out.push({ path: pointerOf([...path, "nodes"]), code: "exits" })
  else if (!action.nodes.every(isNodeSelector))
    out.push({ path: pointerOf([...path, "nodes"]), code: "node" })
  if (!(action.via ?? []).every(isNodeSelector))
    out.push({ path: pointerOf([...path, "via"]), code: "node" })
}

function rulesProblems(
  out: RoutingProblem[],
  path: JsonPath,
  rules: readonly EgressRule[]
) {
  rules.forEach((rule, index) => {
    const values = matchValues(rule.match)
    const at = [...path, index]
    if (values.length === 0)
      out.push({ path: pointerOf([...at, "match"]), code: "match" })
    else if (values.some(badDestination))
      out.push({ path: pointerOf([...at, "match"]), code: "destination" })
    actionProblems(out, [...at, "action"], rule.action)
  })
}

export function routingProblems(
  routing: RoutingConfiguration
): RoutingProblem[] {
  const out: RoutingProblem[] = []
  ;(routing.mesh ?? []).forEach((rule, index) => {
    for (const end of ["src", "dst"] as const) {
      const path = pointerOf(["routing", "mesh", index, end])
      if (rule[end].length === 0) out.push({ path, code: "empty" })
      else if (rule[end].some((value) => selectorKind(value, false) === null))
        out.push({ path, code: "selector" })
    }
    if (!rule.via.every(isNodeSelector))
      out.push({
        path: pointerOf(["routing", "mesh", index, "via"]),
        code: "node",
      })
  })
  const egress = routing.egress ?? {}
  rulesProblems(out, ["routing", "egress", "rules"], egress.rules ?? [])
  for (const [key, profile] of Object.entries(egress.profiles ?? {})) {
    const path = ["routing", "egress", "profiles", key]
    if (!isProfileName(key))
      out.push({ path: pointerOf(path), code: "profileName" })
    rulesProblems(out, [...path, "rules"], profile.rules ?? [])
    actionProblems(out, [...path, "fallback"], profile.fallback)
  }
  ;(egress.assignments ?? []).forEach((assignment, index) => {
    const path = pointerOf([
      "routing",
      "egress",
      "assignments",
      index,
      "target",
    ])
    if (assignment.target.length === 0) out.push({ path, code: "empty" })
    else if (
      assignment.target.some((value) => selectorKind(value, false) === null)
    )
      out.push({ path, code: "selector" })
  })
  if (
    egress.auto?.rules.some(
      (rule) => rule.countries.length === 0 || !rule.countries.every(isCountry)
    )
  )
    out.push({ path: "/routing/egress/auto/rules", code: "country" })
  return out
}
