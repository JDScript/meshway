import type {
  ConfigurationIssue,
  IssueCode,
  ResourceProblem,
} from "@meshway/control-client"

import { byteSize } from "@/lib/format"
import { m } from "@/paraglide/messages.js"
import { getLocale } from "@/paraglide/runtime.js"

import type { Checked } from "./draft-state"
import { pointerSegments } from "./jsonc"
import { requirementText } from "./preparation"

// What the controller says about a configuration, in words: an issue by
// its closed code and string parameters (problems.md), a transport
// declaration's reason, and a rule set's problem. The English `message`
// an issue carries is never shown.

/** A rule set's problem, as its check or a refusal reports it. */
export const RESOURCE_PROBLEM: Record<ResourceProblem, () => string> = {
  download_failed: () => m.configuration_ruleset_problem_download_failed(),
  invalid_source: () => m.configuration_ruleset_problem_invalid_source(),
  too_large: () => m.configuration_ruleset_problem_too_large(),
  conversion_failed: () => m.configuration_ruleset_problem_conversion_failed(),
  configuration_changed: () =>
    m.configuration_ruleset_problem_configuration_changed(),
  budget_exceeded: () => m.configuration_ruleset_problem_budget_exceeded(),
  not_ready: () => m.configuration_ruleset_problem_not_ready(),
  validation_failed: () => m.configuration_ruleset_problem_validation_failed(),
  unknown_rule_set: () => m.configuration_ruleset_problem_unknown_rule_set(),
  rule_set_budget: () => m.configuration_ruleset_problem_rule_set_budget(),
  signature_invalid: () => m.configuration_ruleset_problem_signature_invalid(),
  catalog_regressed: () => m.configuration_ruleset_problem_catalog_regressed(),
  integrity_failed: () => m.configuration_ruleset_problem_integrity_failed(),
}

const isResourceProblem = (value: string): value is ResourceProblem =>
  Object.hasOwn(RESOURCE_PROBLEM, value)

/** A transport declaration's reason (`invalid_transport.reason`). */
const TRANSPORT_REASON: Record<string, () => string> = {
  invalid_host: () => m.configuration_transport_invalid_host(),
  keep_alive_count: () => m.configuration_transport_keep_alive_count(),
  tls_version_order: () => m.configuration_transport_tls_version_order(),
  invalid_alpn: () => m.configuration_transport_invalid_alpn(),
  provided_tls_incomplete: () =>
    m.configuration_transport_provided_tls_incomplete(),
  empty_tls_material: () => m.configuration_transport_empty_tls_material(),
  vless_flow: () => m.configuration_transport_vless_flow(),
  invalid_vless_uuid: () => m.configuration_transport_invalid_vless_uuid(),
  tcp_stream_websocket: () => m.configuration_transport_tcp_stream_websocket(),
  websocket_alpn: () => m.configuration_transport_websocket_alpn(),
  empty_trojan_password: () =>
    m.configuration_transport_empty_trojan_password(),
  material_required: () => m.configuration_transport_material_required(),
  local_address: () => m.configuration_transport_local_address(),
  repeated_transport: () => m.configuration_transport_repeated_transport(),
  retained_secret_unavailable: () =>
    m.configuration_transport_retained_secret_unavailable(),
}

/** How an issue names nodes: a Node ID or `node:<id>` as its hostname, else as written. */
export type NodeNamer = (value: string) => string

/** Names nodes by the hostnames a configuration gives them. */
export const nodeNamer =
  (nodes: Readonly<Record<string, { hostname: string }>>): NodeNamer =>
  (value) => {
    const id = value.startsWith("node:") ? value.slice("node:".length) : value
    return (Object.hasOwn(nodes, id) ? nodes[id]?.hostname : undefined) ?? value
  }

function sizeText(value: string | number | undefined): string {
  const bytes = Number(value)
  return Number.isFinite(bytes) ? byteSize(bytes) : String(value ?? "?")
}

/** Two sizes said so they differ: in bytes when rounding would make them equal. */
export function sizesApart(
  value: string | number | undefined,
  limit: string | number | undefined
): [string, string] {
  const [size, max] = [sizeText(value), sizeText(limit)]
  if (size !== max) return [size, max]
  const bytes = (text: string | number | undefined) => {
    const count = Number(text)
    return Number.isFinite(count)
      ? m.configuration_bytes({ count: count.toLocaleString(getLocale()) })
      : String(text ?? "?")
  }
  return [bytes(value), bytes(limit)]
}

/** A rule or assignment index (0-based) as the forms number them. */
const ordinal = (value: string | undefined) => {
  const index = Number(value)
  return Number.isInteger(index) ? String(index + 1) : (value ?? "?")
}

type Params = Readonly<Record<string, string>>

const TEXT: Record<IssueCode, (p: Params, node: NodeNamer) => string> = {
  invalid_jsonc: (p) =>
    m.configuration_issue_invalid_jsonc({
      line: p.line ?? "?",
      column: p.column ?? "?",
    }),
  invalid_shape: (p) =>
    m.configuration_issue_invalid_shape({
      line: p.line ?? "?",
      column: p.column ?? "?",
    }),
  incomplete_version: () => m.configuration_issue_incomplete_version(),
  unknown_user: (p) =>
    m.configuration_issue_unknown_user({ user: p.user ?? "?" }),
  invalid_group_name: (p) =>
    m.configuration_issue_invalid_group_name({ name: p.name ?? "?" }),
  invalid_tag_name: (p) =>
    m.configuration_issue_invalid_tag_name({ name: p.name ?? "?" }),
  unknown_group: (p) =>
    m.configuration_issue_unknown_group({ name: p.name ?? "?" }),
  unknown_tag: (p) =>
    m.configuration_issue_unknown_tag({ name: p.name ?? "?" }),
  duplicate_tag: (p) =>
    m.configuration_issue_duplicate_tag({ name: p.name ?? "?" }),
  identity_membership_missing: (p) =>
    m.configuration_issue_identity_membership_missing({ user: p.user ?? "?" }),
  empty_tag_identity: () => m.configuration_issue_empty_tag_identity(),
  unknown_node: (p, node) =>
    m.configuration_issue_unknown_node({ node: node(p.node ?? "?") }),
  unknown_node_selector: (p, node) =>
    m.configuration_issue_unknown_node_selector({ node: node(p.node ?? "?") }),
  retained_membership_inactive: () =>
    m.configuration_issue_retained_membership_inactive(),
  empty_selector_set: () => m.configuration_issue_empty_selector_set(),
  bidirectional_requires_mesh: () =>
    m.configuration_issue_bidirectional_requires_mesh(),
  invalid_port_set: () => m.configuration_issue_invalid_port_set(),
  invalid_domain_suffix: (p) =>
    m.configuration_issue_invalid_domain_suffix({ value: p.value ?? "?" }),
  invalid_domain: (p) =>
    m.configuration_issue_invalid_domain({ value: p.value ?? "?" }),
  noncanonical_cidr: (p) =>
    m.configuration_issue_noncanonical_cidr({ value: p.value ?? "?" }),
  invalid_dns_domain: () => m.configuration_issue_invalid_dns_domain(),
  invalid_node_dns_name: () => m.configuration_issue_invalid_node_dns_name(),
  dns_override_requires_default: () =>
    m.configuration_issue_dns_override_requires_default(),
  unknown_resolver: (p) =>
    m.configuration_issue_unknown_resolver({ name: p.name ?? "?" }),
  invalid_resolver: () => m.configuration_issue_invalid_resolver(),
  empty_resolver_set: () => m.configuration_issue_empty_resolver_set(),
  duplicate_dns_suffix: (p) =>
    m.configuration_issue_duplicate_dns_suffix({ value: p.value ?? "?" }),
  invalid_dns_suffix: (p) =>
    m.configuration_issue_invalid_dns_suffix({ value: p.value ?? "?" }),
  invalid_hostname: () => m.configuration_issue_invalid_hostname(),
  duplicate_hostname: (p) =>
    m.configuration_issue_duplicate_hostname({ hostname: p.hostname ?? "?" }),
  empty_listener_key: () => m.configuration_issue_empty_listener_key(),
  invalid_transport: (p, node) => {
    if (p.reason === "edge_transport_unavailable") {
      const [section, kind, index] = pointerSegments(p.routingRule ?? "") ?? []
      const ends = { a: node(p.a ?? "?"), b: node(p.b ?? "?") }
      return section === "routing" && kind === "mesh" && index !== undefined
        ? m.configuration_issue_edge_transport_unavailable({
            ...ends,
            index: ordinal(index),
          })
        : m.configuration_issue_edge_transport_unavailable_edge(ends)
    }
    const reason =
      p.reason !== undefined && Object.hasOwn(TRANSPORT_REASON, p.reason)
        ? TRANSPORT_REASON[p.reason]
        : undefined
    return reason
      ? m.configuration_issue_invalid_transport_reason({ reason: reason() })
      : m.configuration_issue_invalid_transport()
  },
  invalid_listener_materials: () =>
    m.configuration_issue_invalid_listener_materials(),
  overlapping_pools: () => m.configuration_issue_overlapping_pools(),
  invalid_pool: (p) =>
    m.configuration_issue_invalid_pool({ family: familyText(p.family) }),
  address_outside_pool: (p) =>
    m.configuration_issue_address_outside_pool({ address: p.address ?? "?" }),
  duplicate_address: (p) =>
    m.configuration_issue_duplicate_address({ address: p.address ?? "?" }),
  unknown_reassignment_target: (p, node) =>
    m.configuration_issue_unknown_reassignment_target({
      node: node(p.node ?? "?"),
    }),
  no_address_available: (p) =>
    m.configuration_issue_no_address_available({
      family: familyText(p.family),
    }),
  repeated_required_node: (p, node) =>
    m.configuration_issue_repeated_required_node({ node: node(p.node ?? "?") }),
  empty_exit_set: () => m.configuration_issue_empty_exit_set(),
  repeated_exit: (p, node) =>
    m.configuration_issue_repeated_exit({ node: node(p.node ?? "?") }),
  exit_not_enabled: (p, node) =>
    m.configuration_issue_exit_not_enabled({ node: node(p.node ?? "?") }),
  invalid_destination_cidr: (p) =>
    m.configuration_issue_invalid_destination_cidr({ value: p.value ?? "?" }),
  empty_route_match: () => m.configuration_issue_empty_route_match(),
  unknown_rule_set: (p) =>
    m.configuration_issue_unknown_rule_set({ key: p.key ?? "?" }),
  invalid_rule_source: () => m.configuration_issue_invalid_rule_source(),
  empty_rule_set_key: () => m.configuration_issue_empty_rule_set_key(),
  invalid_rule_set_key: (p) =>
    m.configuration_issue_invalid_rule_set_key({ key: p.key ?? "?" }),
  unknown_rule_set_provider: (p) =>
    m.configuration_issue_unknown_rule_set_provider({
      provider: p.provider ?? "?",
    }),
  invalid_rule_set_provider: (p) =>
    m.configuration_issue_invalid_rule_set_provider({ field: p.field ?? "?" }),
  rule_set_partition_limit: (p) =>
    m.configuration_issue_rule_set_partition_limit({
      key: p.key ?? "?",
      prefixes: p.prefixes ?? "?",
      max: p.maxPrefixes ?? "?",
    }),
  empty_inline_rule_set: () => m.configuration_issue_empty_inline_rule_set(),
  rule_set_budget: (p) => {
    if (p.maxSets !== undefined)
      return m.configuration_issue_rule_set_budget_sets({ count: p.maxSets })
    if (p.maxBytes !== undefined && p.maxEntries !== undefined)
      return m.configuration_issue_rule_set_budget_content({
        size: sizeText(p.maxBytes),
        entries: p.maxEntries,
      })
    if (p.maxBytes !== undefined)
      return m.configuration_issue_rule_set_budget_size({
        size: sizeText(p.maxBytes),
      })
    return m.configuration_issue_rule_set_budget()
  },
  rule_set_unavailable: (p) =>
    p.reason !== undefined && isResourceProblem(p.reason)
      ? m.configuration_issue_rule_set_unavailable_reason({
          reason: RESOURCE_PROBLEM[p.reason](),
        })
      : m.configuration_issue_rule_set_unavailable(),
  rule_set_timeout: (p) =>
    p.seconds !== undefined
      ? m.configuration_issue_rule_set_timeout_seconds({ seconds: p.seconds })
      : m.configuration_issue_rule_set_timeout(),
  reserved_profile_name: () => m.configuration_issue_reserved_profile_name(),
  unknown_profile: (p, node) =>
    m.configuration_issue_unknown_profile({
      name: profileText(p.name ?? "?", node),
    }),
  default_not_allowed: (p, node) =>
    m.configuration_issue_default_not_allowed({
      profile: profileText(p.profile ?? "?", node),
    }),
  auto_target_not_allowed: (p, node) =>
    p.node !== undefined
      ? m.configuration_issue_auto_target_not_allowed_node({
          profile: profileText(p.profile ?? "?", node),
          node: node(p.node),
        })
      : m.configuration_issue_auto_target_not_allowed({
          profile: profileText(p.profile ?? "?", node),
        }),
  auto_target_unknown: (p, node) =>
    m.configuration_issue_auto_target_unknown({
      profile: profileText(p.profile ?? "?", node),
    }),
  conflicting_assignments: (p) =>
    m.configuration_issue_conflicting_assignments({
      index: ordinal(p.other),
    }),
  invalid_country_code: (p) =>
    m.configuration_issue_invalid_country_code({ value: p.value ?? "?" }),
  exit_outside_access: (p, node) =>
    m.configuration_issue_exit_outside_access({
      node: node(p.node ?? "?"),
      destination: (p.destination ?? "?").replace(/^domain:/, ""),
    }),
  global_exit_requires_internet: (p, node) =>
    m.configuration_issue_global_exit_requires_internet({
      node: node(p.node ?? "?"),
      profile: profileText(p.profile ?? "?", node),
    }),
  exit_fallback_requires_internet: (p, node) =>
    m.configuration_issue_exit_fallback_requires_internet({
      node: node(p.node ?? "?"),
      profile: profileText(p.profile ?? "?", node),
    }),
  netmap_limit: (p, node) => {
    const [size, max] = sizesApart(p.bytes, p.maxBytes)
    return m.configuration_issue_netmap_limit({
      node: node(p.node ?? "?"),
      size,
      max,
    })
  },
  netmap_budget: () => m.configuration_issue_netmap_budget(),
  shadowed_rule: (p) =>
    m.configuration_issue_shadowed_rule({ index: ordinal(p.by) }),
}

function familyText(value: string | undefined): string {
  return value === "ipv4" ? "IPv4" : value === "ipv6" ? "IPv6" : (value ?? "?")
}

/** A profile as the routing form names it: the built-ins by their labels. */
export function profileText(name: string, node: NodeNamer): string {
  if (name === "mesh") return m.configuration_routing_profile_mesh()
  if (name === "auto") return m.configuration_routing_profile_auto()
  if (name.startsWith("global:"))
    return m.configuration_routing_profile_global({
      node: node(name.slice("global:".length)),
    })
  return name
}

/** An issue (or warning) in words, nodes by hostname. */
export const issueText = (
  issue: Pick<ConfigurationIssue, "code" | "params">,
  node: NodeNamer
): string => TEXT[issue.code](issue.params, node)

export type Severity = "error" | "warning"

/** One thing a check said, at its pointer, in words. */
export type Said = { path: string; message: string; severity: Severity }

/** What a check said: why it refused (issues, requirements), or what it accepted but warned of. */
export function saidOf(
  checked: Pick<Checked, "issues" | "requirements" | "warnings"> | null,
  node: NodeNamer
): Said[] {
  if (!checked) return []
  return [
    ...checked.issues.map((issue) => ({
      path: issue.path,
      message: issueText(issue, node),
      severity: "error" as const,
    })),
    ...checked.requirements.map((requirement) => ({
      path: requirement.path,
      message: requirementText(requirement),
      severity: "error" as const,
    })),
    ...checked.warnings.map((warning) => ({
      path: warning.path,
      message: issueText(warning, node),
      severity: "warning" as const,
    })),
  ]
}
