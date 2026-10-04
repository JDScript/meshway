import type {
  AccessRule,
  AddressPools,
  ConfigurationOperation,
  ConfigurationView,
  DefaultsUpdate,
  DialerSetUpdate,
  DialerSetView,
  DnsConfiguration,
  RoutingConfiguration,
  TopologyDefault,
  TopologyRule,
} from "@meshway/control-client"

import { sameRouting, writtenRouting } from "./routing.ts"
import { sameRuleSets, writtenRuleSet, type RuleSetMap } from "./rule-sets.ts"

// What the section forms put in the draft: one typed operation per edited
// part, holding that part's final value (`replaceAccess` with every rule,
// not one per click). Editing a part again replaces its operation; a value
// equal to the base drops it. Values are compared the way the controller
// reads them: access sets and group members are sets; omitted remarks and
// directions are their defaults.

export type GroupMap = Record<string, string[]>

/** The parts the forms edit, by what they hold. */
export type FormValues = {
  access: AccessRule[]
  topology: TopologyRule[]
  topologyDefault: TopologyDefault
  groups: GroupMap
  tagsOwners: GroupMap
  dns: DnsConfiguration
  addressPools: AddressPools
  routing: RoutingConfiguration
  ruleSets: RuleSetMap
}

export type FormPart = keyof FormValues

type Part<V> = {
  /** The operation from `value`; `base` supplies what the operation must restate. */
  operation: (value: V, base: ConfigurationView) => ConfigurationOperation
  /** The part's value in an operation of its kind; undefined in any other. */
  read: (operation: ConfigurationOperation) => V | undefined
  base: (view: ConfigurationView) => V
  same: (a: V, b: V) => boolean
}

const asSet = (values: readonly string[]) => [...new Set(values)].sort()
const sameList = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((value, index) => value === b[index])
const sameSet = (a: readonly string[], b: readonly string[]) =>
  sameList(asSet(a), asSet(b))

function sameMap(
  a: Readonly<Record<string, readonly string[]>>,
  b: Readonly<Record<string, readonly string[]>>,
  values: (a: readonly string[], b: readonly string[]) => boolean
) {
  const keys = Object.keys(a)
  if (keys.length !== Object.keys(b).length) return false
  return keys.every((key) => {
    const other = b[key]
    const value = a[key]
    return other !== undefined && value !== undefined && values(value, other)
  })
}

/** A rule as the form writes it: the remark and direction only when not their defaults. */
export function accessRule(rule: AccessRule): AccessRule {
  const out: AccessRule = { src: rule.src, dst: rule.dst, ip: rule.ip }
  if (rule.remark) out.remark = rule.remark
  if (rule.bidirectional) out.bidirectional = true
  return out
}

const sameAccessRule = (a: AccessRule, b: AccessRule) =>
  (a.remark ?? "") === (b.remark ?? "") &&
  (a.bidirectional ?? false) === (b.bidirectional ?? false) &&
  sameSet(a.src, b.src) &&
  sameSet(a.dst, b.dst) &&
  sameSet(a.ip, b.ip)

export function topologyRule(rule: TopologyRule): TopologyRule {
  const out: TopologyRule = { action: rule.action, a: rule.a, b: rule.b }
  if (rule.remark) out.remark = rule.remark
  return out
}

const sameTopologyRule = (a: TopologyRule, b: TopologyRule) =>
  (a.remark ?? "") === (b.remark ?? "") &&
  a.action === b.action &&
  a.a === b.a &&
  a.b === b.b

const sameRules =
  <R>(same: (a: R, b: R) => boolean) =>
  (a: readonly R[], b: readonly R[]) =>
    a.length === b.length &&
    a.every((rule, index) => {
      const other = b[index]
      return other !== undefined && same(rule, other)
    })

/**
 * The defaults as an update: `DefaultsUpdate` restates the dialers, so they
 * are the view's (a VLESS dialer's WebSocket headers are omitted: kept).
 */
export function defaultsUpdate(
  view: ConfigurationView,
  topology: TopologyDefault
): DefaultsUpdate {
  return {
    access: view.defaults.access,
    topology,
    dialers: dialerUpdate(view.defaults.dialers),
  }
}

export function dialerUpdate(view: DialerSetView): DialerSetUpdate {
  const out: DialerSetUpdate = {}
  if (view.native_udp) out.native_udp = view.native_udp
  if (view.native_tcp) out.native_tcp = view.native_tcp
  if (view.trojan) out.trojan = view.trojan
  if (view.vless) {
    const { websocketHeaderNames, ...vless } = view.vless
    void websocketHeaderNames
    out.vless = vless
  }
  return out
}

const DNS_DEFAULTS = {
  magicDNS: true,
  overrideLocal: false,
} as const

function sameDns(a: DnsConfiguration, b: DnsConfiguration): boolean {
  const servers = (dns: DnsConfiguration) =>
    Object.entries(dns.servers ?? {})
      .map(([key, server]) =>
        JSON.stringify([
          key,
          server.type,
          server.server,
          server.serverPort ?? 53,
          server.useWithExit ?? false,
        ])
      )
      .sort()
  return (
    a.domain === b.domain &&
    (a.magicDNS ?? DNS_DEFAULTS.magicDNS) ===
      (b.magicDNS ?? DNS_DEFAULTS.magicDNS) &&
    (a.overrideLocal ?? DNS_DEFAULTS.overrideLocal) ===
      (b.overrideLocal ?? DNS_DEFAULTS.overrideLocal) &&
    sameList(a.default ?? [], b.default ?? []) &&
    sameList(a.searchDomains ?? [], b.searchDomains ?? []) &&
    sameMap(a.routes ?? {}, b.routes ?? {}, sameList) &&
    sameList(servers(a), servers(b))
  )
}

const PARTS: { [P in FormPart]: Part<FormValues[P]> } = {
  access: {
    operation: (access) => ({
      type: "replaceAccess",
      access: access.map(accessRule),
    }),
    read: (op) => (op.type === "replaceAccess" ? op.access : undefined),
    base: (view) => view.access,
    same: sameRules(sameAccessRule),
  },
  topology: {
    operation: (topology) => ({
      type: "replaceTopology",
      topology: topology.map(topologyRule),
    }),
    read: (op) => (op.type === "replaceTopology" ? op.topology : undefined),
    base: (view) => view.topology,
    same: sameRules(sameTopologyRule),
  },
  topologyDefault: {
    operation: (topology, base) => ({
      type: "replaceDefaults",
      defaults: defaultsUpdate(base, topology),
    }),
    read: (op) =>
      op.type === "replaceDefaults" ? op.defaults.topology : undefined,
    base: (view) => view.defaults.topology,
    same: (a, b) => a === b,
  },
  groups: {
    operation: (groups) => ({ type: "replaceGroups", groups }),
    read: (op) => (op.type === "replaceGroups" ? op.groups : undefined),
    base: (view) => view.groups,
    same: (a, b) => sameMap(a, b, sameSet),
  },
  tagsOwners: {
    operation: (tagsOwners) => ({ type: "replaceTagsOwners", tagsOwners }),
    read: (op) => (op.type === "replaceTagsOwners" ? op.tagsOwners : undefined),
    base: (view) => view.tagsOwners,
    same: (a, b) => sameMap(a, b, sameSet),
  },
  dns: {
    operation: (dns) => ({ type: "replaceDns", dns }),
    read: (op) => (op.type === "replaceDns" ? op.dns : undefined),
    base: (view) => view.dns,
    same: sameDns,
  },
  addressPools: {
    operation: (pools) => ({
      type: "setAddressPools",
      addressPools: { ipv4: pools.ipv4, ipv6: pools.ipv6 },
    }),
    read: (op) =>
      op.type === "setAddressPools" &&
      op.addressPools.ipv4 !== undefined &&
      op.addressPools.ipv6 !== undefined
        ? { ipv4: op.addressPools.ipv4, ipv6: op.addressPools.ipv6 }
        : undefined,
    base: (view) => view.addressPools,
    same: (a, b) => sameList(a.ipv4, b.ipv4) && sameList(a.ipv6, b.ipv6),
  },
  routing: {
    operation: (routing) => ({
      type: "replaceRouting",
      routing: writtenRouting(routing),
    }),
    read: (op) => (op.type === "replaceRouting" ? op.routing : undefined),
    base: (view) => view.routing,
    same: sameRouting,
  },
  ruleSets: {
    operation: (ruleSets) => ({
      type: "replaceRuleSets",
      ruleSets: Object.fromEntries(
        Object.entries(ruleSets).map(([key, set]) => [key, writtenRuleSet(set)])
      ),
    }),
    read: (op) => (op.type === "replaceRuleSets" ? op.ruleSets : undefined),
    base: (view) => view.ruleSets,
    same: sameRuleSets,
  },
}

/** The parts, in the order their operations are sent. */
export const FORM_PARTS: readonly FormPart[] = [
  "access",
  "topology",
  "topologyDefault",
  "groups",
  "tagsOwners",
  "dns",
  "addressPools",
  "ruleSets",
  "routing",
]

const partOf = (op: ConfigurationOperation): FormPart | null =>
  FORM_PARTS.find((part) => PARTS[part].read(op) !== undefined) ?? null

/** A part's value in the draft: its operation's, else the base's. */
export function valueOf<P extends FormPart>(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  part: P
): FormValues[P] {
  const described: Part<FormValues[P]> = PARTS[part]
  for (const op of ops) {
    const value = described.read(op)
    if (value !== undefined) return value
  }
  return described.base(base)
}

/** Whether the draft's operations change `part`. */
export const edits = (
  ops: readonly ConfigurationOperation[],
  part: FormPart
): boolean => ops.some((op) => PARTS[part].read(op) !== undefined)

/**
 * The operations with `part` set to `value`: its operation replaced, or
 * dropped when `value` is the base's. Operations in a fixed order.
 */
export function withValue<P extends FormPart>(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView,
  part: P,
  value: FormValues[P]
): ConfigurationOperation[] {
  const described: Part<FormValues[P]> = PARTS[part]
  const rest = ops.filter((op) => described.read(op) === undefined)
  const next = described.same(value, described.base(base))
    ? rest
    : [...rest, described.operation(value, base)]
  const rank = (op: ConfigurationOperation) => {
    const at = partOf(op)
    return at === null ? FORM_PARTS.length : FORM_PARTS.indexOf(at)
  }
  return next.sort((a, b) => rank(a) - rank(b))
}

/** The configuration section a part is in, where its diagnostics point. */
export const sectionOfPart: Record<FormPart, string> = {
  access: "access",
  topology: "topology",
  topologyDefault: "defaults",
  groups: "groups",
  tagsOwners: "tagsOwners",
  dns: "dns",
  addressPools: "addressPools",
  routing: "routing",
  ruleSets: "ruleSets",
}
