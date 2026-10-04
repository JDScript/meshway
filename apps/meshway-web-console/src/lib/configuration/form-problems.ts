import type {
  ConfigurationOperation,
  ConfigurationView,
} from "@meshway/control-client"

import { isIpv4, isIpv6 } from "@/lib/ip"

import { dialerProblems } from "./dialer-model"
import { pointerOf, pointerSegments, type JsonPath } from "./jsonc"
import { nodeProblems } from "./node-problems"
import {
  edits,
  FORM_PARTS,
  valueOf,
  type FormPart,
  type FormValues,
} from "./operations"
import { routingProblems, type RoutingCode } from "./routing"
import { ruleSetProblems, type RuleSetCode } from "./rule-sets"
import {
  cidrFamily,
  isDomain,
  isEmail,
  isNamed,
  isProtocol,
  selectorKind,
} from "./selectors"

// What a form's value cannot be saved with, found before asking the
// controller: empty sets the reference requires, values not in the form it
// states. Placed by the JSON Pointer the controller would use, so the form
// marks the same field either way. The controller checks everything again.

export type LocalCode =
  | "empty"
  | "selector"
  | "protocol"
  | "bidirectional"
  | "member"
  | "owner"
  | "domain"
  | "resolver"
  | "server"
  | "default"
  | "cidr"
  | RoutingCode
  | RuleSetCode
  | "dialer"
  | "hostname"
  | "hostname_taken"
  | "tags"
  | "tag"
  | "address"
  | "address_pool"
  | "address_taken"

export type LocalProblem = { path: string; code: LocalCode }

const at = (path: JsonPath) => pointerOf(path)

function selectorSet(
  out: LocalProblem[],
  path: JsonPath,
  values: readonly string[],
  external: boolean
) {
  if (values.length === 0) out.push({ path: at(path), code: "empty" })
  else if (values.some((value) => selectorKind(value, external) === null))
    out.push({ path: at(path), code: "selector" })
}

const isExternal = (value: string) => {
  const kind = selectorKind(value, true)
  return kind === "internet" || kind === "domain" || kind === "cidr"
}

const PROBLEMS: {
  [P in FormPart]: (value: FormValues[P]) => LocalProblem[]
} = {
  access(rules) {
    const out: LocalProblem[] = []
    rules.forEach((rule, index) => {
      selectorSet(out, ["access", index, "src"], rule.src, false)
      selectorSet(out, ["access", index, "dst"], rule.dst, true)
      if (rule.ip.length === 0)
        out.push({ path: at(["access", index, "ip"]), code: "empty" })
      else if (!rule.ip.every(isProtocol))
        out.push({ path: at(["access", index, "ip"]), code: "protocol" })
      if (rule.bidirectional && rule.dst.some(isExternal))
        out.push({ path: at(["access", index]), code: "bidirectional" })
    })
    return out
  },
  topology(rules) {
    const out: LocalProblem[] = []
    rules.forEach((rule, index) => {
      for (const end of ["a", "b"] as const)
        if (selectorKind(rule[end], false) === null)
          out.push({ path: at(["topology", index, end]), code: "selector" })
    })
    return out
  },
  defaults: (value) =>
    dialerProblems(value.dialers).map((protocol) => ({
      path: at(["defaults", "dialers", protocol]),
      code: "dialer",
    })),
  groups(groups) {
    return Object.entries(groups)
      .filter(([, members]) => !members.every(isEmail))
      .map(([key]) => ({ path: at(["groups", key]), code: "member" }))
  },
  tagsOwners(owners) {
    return Object.entries(owners)
      .filter(
        ([, values]) =>
          !values.every((value) => isEmail(value) || isNamed(value, "group:"))
      )
      .map(([key]) => ({ path: at(["tagsOwners", key]), code: "owner" }))
  },
  dns(dns) {
    const out: LocalProblem[] = []
    const servers = dns.servers ?? {}
    if (!isDomain(dns.domain))
      out.push({ path: at(["dns", "domain"]), code: "domain" })
    for (const [key, server] of Object.entries(servers))
      if (!isIpv4(server.server) && !isIpv6(server.server))
        out.push({ path: at(["dns", "servers", key]), code: "server" })
    const defaults = dns.default ?? []
    if (defaults.some((key) => !Object.hasOwn(servers, key)))
      out.push({ path: at(["dns", "default"]), code: "resolver" })
    else if ((dns.overrideLocal ?? false) && defaults.length === 0)
      out.push({ path: at(["dns", "default"]), code: "default" })
    for (const [suffix, keys] of Object.entries(dns.routes ?? {})) {
      const path = at(["dns", "routes", suffix])
      if (!isDomain(suffix)) out.push({ path, code: "domain" })
      else if (
        keys.length === 0 ||
        keys.some((key) => !Object.hasOwn(servers, key))
      )
        out.push({ path, code: "resolver" })
    }
    if (!(dns.searchDomains ?? []).every(isDomain))
      out.push({ path: at(["dns", "searchDomains"]), code: "domain" })
    return out
  },
  addressPools(pools) {
    const out: LocalProblem[] = []
    for (const family of ["ipv4", "ipv6"] as const)
      if (!pools[family].every((value) => cidrFamily(value) === family))
        out.push({ path: at(["addressPools", family]), code: "cidr" })
    return out
  },
  routing: routingProblems,
  ruleSets: ruleSetProblems,
}

export function partProblems<P extends FormPart>(
  part: P,
  value: FormValues[P]
): LocalProblem[] {
  const check: (value: FormValues[P]) => LocalProblem[] = PROBLEMS[part]
  return check(value)
}

/** What the parts and the nodes the operations edit cannot be saved with. */
export const operationsProblems = (
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView
): LocalProblem[] => [
  ...FORM_PARTS.filter((part) => edits(ops, part)).flatMap((part) =>
    partProblems(part, valueOf(ops, base, part))
  ),
  ...nodeProblems(ops, base),
]

/** `pointer` is `prefix` or inside it. */
export function within(pointer: string, prefix: string): boolean {
  const inner = pointerSegments(pointer)
  const outer = pointerSegments(prefix)
  if (inner === null || outer === null || inner.length < outer.length)
    return false
  return outer.every((segment, index) => inner[index] === segment)
}
