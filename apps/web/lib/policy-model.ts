import type {
  FlowQuery,
  NodeSummary,
  PolicyDiagnostic,
  RouteRule,
  RoutingPolicy,
} from "@meshway/control-client"

// Editor-side form model for the routing policy. It mirrors the generated
// RoutingPolicy DTO field for field; only text inputs keep their raw string so
// typing is not disrupted. `toPolicy` produces the exact DTO that is sent.

export type DefaultAction = RoutingPolicy["default_action"]
export type DestinationScope = RouteRule["destination_scope"]
export type DestinationSelector = RouteRule["destination_any"][number]
export type DestinationKind = DestinationSelector["kind"]
export type ActionKind = RouteRule["action"]["kind"]

/** Controller compiler caps (crates/policy). The editor never claims more. */
export const POLICY_LIMITS = {
  rules: 128,
  via: 8,
  selectorsPerRule: 32,
  policyBytes: 128 * 1024,
} as const

export type SelectorDraft = { key: string; kind: DestinationKind; value: string }

export type RuleDraft = {
  key: string
  id: string
  sourceNodeIds: string[]
  sourceCountries: string
  scope: DestinationScope
  destinations: SelectorDraft[]
  action: ActionKind
  /** Ordered intermediate node IDs; "" is a picker not yet chosen. */
  via: string[]
  /** "" means no explicit exit. */
  exitNodeId: string
}

export type PolicyDraft = { defaultAction: DefaultAction; rules: RuleDraft[] }

export const destinationKindOptions: {
  value: DestinationKind
  label: string
  placeholder: string
}[] = [
  { value: "node", label: "Mesh 节点", placeholder: "" },
  { value: "domain_suffix", label: "域名后缀", placeholder: "example.com" },
  { value: "cidr", label: "CIDR", placeholder: "203.0.113.0/24 或 2001:db8::/32" },
  { value: "country", label: "国家", placeholder: "JP" },
]

export function parseCountryList(text: string): string[] {
  return text
    .split(/[\s,，、]+/)
    .filter((part) => part.length > 0)
    .map((part) => part.toUpperCase())
}

export function fromPolicy(policy: RoutingPolicy): PolicyDraft {
  return {
    defaultAction: policy.default_action,
    rules: policy.rules.map((rule, index) => ({
      key: `saved-${index}`,
      id: rule.id,
      sourceNodeIds: [...rule.source.node_ids],
      sourceCountries: rule.source.countries.join(", "),
      scope: rule.destination_scope,
      destinations: rule.destination_any.map((selector, selectorIndex) => ({
        key: `saved-${index}-${selectorIndex}`,
        kind: selector.kind,
        value: selector.value,
      })),
      action: rule.action.kind,
      via: [...rule.action.via],
      exitNodeId: rule.action.exit_node_id ?? "",
    })),
  }
}

export function toRule(rule: RuleDraft): RouteRule {
  return {
    id: rule.id.trim(),
    source: {
      node_ids: [...rule.sourceNodeIds],
      countries: parseCountryList(rule.sourceCountries),
    },
    destination_scope: rule.scope,
    destination_any: rule.destinations.map(({ kind, value }) => ({
      kind,
      value: value.trim(),
    })),
    action: {
      kind: rule.action,
      via: [...rule.via],
      exit_node_id: rule.exitNodeId === "" ? null : rule.exitNodeId,
    },
  }
}

export function toPolicy(draft: PolicyDraft): RoutingPolicy {
  return {
    default_action: draft.defaultAction,
    rules: draft.rules.map(toRule),
  }
}

export function policyBytes(policy: RoutingPolicy): number {
  return new TextEncoder().encode(JSON.stringify(policy)).length
}

// Structural identity. Policies arrive from the generated SDK (whose parsed
// objects follow the schema's key order) and from `toPolicy` (which follows
// the editor's order), so identity must not depend on object key order. The
// projections below rebuild every object with one fixed key order and keep
// every array in its semantic order: rules, via hops and selectors are
// ordered and are never sorted.

function canonicalRule(rule: RouteRule): RouteRule {
  return {
    id: rule.id,
    source: {
      node_ids: [...rule.source.node_ids],
      countries: [...rule.source.countries],
    },
    destination_scope: rule.destination_scope,
    destination_any: rule.destination_any.map((selector) => ({
      kind: selector.kind,
      value: selector.value,
    })),
    action: {
      kind: rule.action.kind,
      via: [...rule.action.via],
      exit_node_id: rule.action.exit_node_id,
    },
  }
}

export function canonicalPolicy(policy: RoutingPolicy): RoutingPolicy {
  return {
    default_action: policy.default_action,
    rules: policy.rules.map(canonicalRule),
  }
}

/** Stable identity of a policy, independent of object key order. */
export const policyKey = (policy: RoutingPolicy) => JSON.stringify(canonicalPolicy(policy))

export const ruleKey = (rule: RouteRule) => JSON.stringify(canonicalRule(rule))

export function samePolicy(a: RoutingPolicy, b: RoutingPolicy): boolean {
  return policyKey(a) === policyKey(b)
}

// ---------------------------------------------------------------------------
// Draft vs stored

export type RuleChange = "new" | "modified" | "unchanged"

const SAVED_KEY = /^saved-(\d+)$/

/**
 * Rules loaded from the stored policy keep a `saved-<index>` key, so a rule
 * stays "the same rule" across renames and reorders.
 */
export function ruleChange(rule: RuleDraft, base: RoutingPolicy): RuleChange {
  const match = SAVED_KEY.exec(rule.key)
  const original = match ? base.rules[Number(match[1])] : undefined
  if (!original) return "new"
  return ruleKey(toRule(rule)) === ruleKey(original) ? "unchanged" : "modified"
}

export function removedRuleCount(draft: PolicyDraft, base: RoutingPolicy): number {
  const kept = new Set(
    draft.rules.map((rule) => SAVED_KEY.exec(rule.key)?.[1]).filter((index) => index !== undefined)
  )
  return base.rules.length - kept.size
}

/** A route or reject rule with no source, scope or target restriction. */
export function isCatchAll(rule: RuleDraft): boolean {
  return (
    rule.sourceNodeIds.length === 0 &&
    parseCountryList(rule.sourceCountries).length === 0 &&
    rule.scope === "any" &&
    rule.destinations.length === 0
  )
}

/** Issues the editor can detect without the controller; they block compile. */
export type LocalIssue = { ruleKey: string | null; field: string; message: string }

const COUNTRY = /^[A-Za-z]{2}$/

export function validateDraft(draft: PolicyDraft): LocalIssue[] {
  const issues: LocalIssue[] = []
  if (draft.rules.length > POLICY_LIMITS.rules) {
    issues.push({
      ruleKey: null,
      field: "rules",
      message: `最多 ${POLICY_LIMITS.rules} 条规则。`,
    })
  }
  const seen = new Map<string, number>()
  for (const rule of draft.rules) {
    const id = rule.id.trim()
    seen.set(id, (seen.get(id) ?? 0) + 1)
  }
  for (const rule of draft.rules) {
    const add = (field: string, message: string) =>
      issues.push({ ruleKey: rule.key, field, message })
    const id = rule.id.trim()
    if (!id) add("id", "规则 ID 不能为空。")
    else if ((seen.get(id) ?? 0) > 1) add("id", "规则 ID 必须唯一。")

    const countries = parseCountryList(rule.sourceCountries)
    countries.forEach((country, index) => {
      if (!COUNTRY.test(country)) {
        add(`source.countries[${index}]`, `「${country}」不是两位字母国家代码。`)
      }
    })
    const selectors =
      rule.sourceNodeIds.length + countries.length + rule.destinations.length
    if (selectors > POLICY_LIMITS.selectorsPerRule) {
      add(
        "selectors",
        `每条规则的源节点、源国家与目标条件合计最多 ${POLICY_LIMITS.selectorsPerRule} 项（当前 ${selectors}）。`
      )
    }
    rule.destinations.forEach((selector, index) => {
      const field = `destination_any[${index}].value`
      const value = selector.value.trim()
      if (!value) {
        add(field, selector.kind === "node" ? "请选择目标节点。" : "请填写条件值。")
      } else if (selector.kind === "country" && !COUNTRY.test(value)) {
        add(field, "国家代码必须是两位字母。")
      }
    })
    if (rule.via.length > POLICY_LIMITS.via) {
      add("action.via", `经由节点最多 ${POLICY_LIMITS.via} 个。`)
    }
    rule.via.forEach((nodeId, index) => {
      if (!nodeId) add(`action.via[${index}]`, "请选择经由节点或移除这一行。")
    })
  }
  const bytes = policyBytes(toPolicy(draft))
  if (bytes > POLICY_LIMITS.policyBytes) {
    issues.push({
      ruleKey: null,
      field: "policy",
      message: `序列化后的策略为 ${Math.ceil(bytes / 1024)} KiB，超过 ${POLICY_LIMITS.policyBytes / 1024} KiB 上限。`,
    })
  }
  return issues
}

/** Friendly name for a compiler field path; the raw path is shown alongside. */
export function fieldLabel(field: string): string {
  const indexed = (pattern: RegExp, label: string) => {
    const match = pattern.exec(field)
    return match ? `${label} #${Number(match[1]) + 1}` : null
  }
  return (
    indexed(/^source\.node_ids\[(\d+)\]$/, "源节点") ??
    indexed(/^source\.countries\[(\d+)\]$/, "源国家") ??
    indexed(/^destination_any\[(\d+)\]/, "目标条件") ??
    indexed(/^action\.via\[(\d+)\]$/, "经由节点") ??
    indexed(/^nodes\[(\d+)\]/, "节点清单") ??
    fieldLabels[field] ??
    field
  )
}

const fieldLabels: Record<string, string | undefined> = {
  id: "规则 ID",
  source: "来源",
  destination_any: "目标条件",
  selectors: "选择条件数量",
  "action.via": "经由节点",
  "action.exit_node_id": "出口节点",
  action: "动作",
  destination_scope: "目标范围",
  rules: "规则列表",
  policy: "整个策略",
  default_action: "默认动作",
  "flow.source_node_id": "预览源节点",
  "flow.destination_node_id": "预览目标节点",
  "flow.destination_ip": "预览目标 IP",
  "flow.domain": "预览目标域名",
  "flow.destination_country": "预览目标国家",
}

export function diagnosticsForRule(
  diagnostics: PolicyDiagnostic[],
  ruleId: string
): PolicyDiagnostic[] {
  return diagnostics.filter((diagnostic) => diagnostic.rule_id === ruleId.trim())
}

// ---------------------------------------------------------------------------
// Preview flow

/** An empty `targetNodeId` means an external (Internet) target. */
export type FlowDraft = {
  sourceNodeId: string
  targetNodeId: string
  domain: string
  ip: string
  country: string
}

export const emptyFlow: FlowDraft = {
  sourceNodeId: "",
  targetNodeId: "",
  domain: "",
  ip: "",
  country: "",
}

const orNull = (value: string) => (value.trim() === "" ? null : value.trim())

/**
 * A mesh target sends only the node ID: domain, IP and country hints typed for
 * an external target are never carried over (mesh country comes from the node).
 */
export function toFlow(flow: FlowDraft): FlowQuery {
  const external = flow.targetNodeId === ""
  return {
    source_node_id: flow.sourceNodeId,
    destination_node_id: external ? null : flow.targetNodeId,
    destination_ip: external ? orNull(flow.ip) : null,
    domain: external ? orNull(flow.domain) : null,
    destination_country: external ? orNull(flow.country.toUpperCase()) : null,
  }
}

export function validateFlow(flow: FlowDraft): string | null {
  if (!flow.sourceNodeId) return "请选择预览源节点。"
  if (flow.targetNodeId !== "") return null
  if (!flow.domain.trim() && !flow.ip.trim()) {
    return "外部目标需要填写域名或 IP；国家只是辅助提示，不能单独作为目标。"
  }
  if (flow.country.trim() && !COUNTRY.test(flow.country.trim())) {
    return "预览目标国家必须是两位字母或留空。"
  }
  return null
}

// ---------------------------------------------------------------------------
// Node references

export type NodeRef =
  | { kind: "active"; node: NodeSummary }
  | { kind: "revoked"; node: NodeSummary }
  | { kind: "unknown"; id: string }

export function resolveNode(nodes: NodeSummary[], id: string): NodeRef {
  const node = nodes.find((candidate) => candidate.id === id)
  if (!node) return { kind: "unknown", id }
  return node.revoked_at === null ? { kind: "active", node } : { kind: "revoked", node }
}

export function nodeRefLabel(ref: NodeRef): string {
  switch (ref.kind) {
    case "active":
      return ref.node.name
    case "revoked":
      return `${ref.node.name}（已撤销）`
    case "unknown":
      return `未知节点 ${ref.id}`
  }
}
