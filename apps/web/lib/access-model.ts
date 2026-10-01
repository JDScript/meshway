import type {
  AccessAction,
  ConnectionNode,
  MeshAccessPolicy,
  MeshAccessRule,
  PolicyDiagnostic,
  TagSnapshot,
} from "@meshway/control-client"

import { jsonEqual } from "./jsonc.ts"
import { parseSelector, selectorListIssues, SELECTORS_MAX } from "./access-selector.ts"

// Editor-side model for endpoint access rules (v2). The draft is the
// generated MeshAccessPolicy itself. Rules are ordered first-match; a
// bidirectional rule also matches the swapped direction at the same position.
// Nothing here evaluates a flow: the controller's flow preview does that.

/** Controller caps for the access policy. The editor never claims more. */
export const ACCESS_LIMITS = {
  rules: 128,
  selectors: SELECTORS_MAX,
  /** Serialized policy cap; the controller enforces its own. */
  policyBytes: 128 * 1024,
  /** Request body cap of the save endpoint (policy plus JSONC source). */
  requestBytes: 512 * 1024,
} as const

/** Contract shape of a rule ID. */
export const RULE_ID_PATTERN = /^[A-Za-z0-9._-]{1,64}$/

export const actionLabels: Record<AccessAction, string> = { allow: "允许", reject: "拒绝" }

export const accessKey = (policy: MeshAccessPolicy) =>
  JSON.stringify([
    policy.default_action,
    policy.rules.map((rule) => [rule.id, rule.action, rule.bidirectional, rule.source, rule.destination]),
  ])

export function sameAccess(a: MeshAccessPolicy, b: MeshAccessPolicy): boolean {
  return accessKey(a) === accessKey(b)
}

// --- Rule IDs ---------------------------------------------------------------

function slug(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 24)
}

/** Short readable label of a selector list for IDs: "all", a tag, a node name. */
function listStem(list: readonly string[], inventory: readonly ConnectionNode[]): string {
  if (list.length !== 1) return list.length === 0 ? "none" : "group"
  const selector = parseSelector(list[0] ?? "")
  switch (selector.kind) {
    case "all":
      return "all"
    case "tag":
      return slug(selector.name) || "tag"
    case "node":
      return slug(inventory.find((entry) => entry.node.id === selector.id)?.node.name ?? "node") || "node"
    case "invalid":
      return "rule"
  }
}

/** A readable, unused ID such as `allow-edge-to-core`, `-2` on collision. */
export function suggestRuleId(
  rule: Pick<MeshAccessRule, "action" | "source" | "destination">,
  taken: readonly string[],
  inventory: readonly ConnectionNode[]
): string {
  const stem = `${rule.action}-${listStem(rule.source, inventory)}-to-${listStem(rule.destination, inventory)}`.slice(0, 60)
  const used = new Set(taken)
  if (!used.has(stem)) return stem
  for (let n = 2; ; n += 1) {
    const candidate = `${stem}-${n}`
    if (!used.has(candidate)) return candidate
  }
}

// --- References -------------------------------------------------------------

export type NodeRef =
  | { kind: "active"; entry: ConnectionNode; keyed: boolean }
  | { kind: "revoked"; entry: ConnectionNode }
  | { kind: "unknown"; id: string }

export function resolveAccessNode(inventory: readonly ConnectionNode[], id: string): NodeRef {
  const entry = inventory.find((candidate) => candidate.node.id === id)
  if (!entry) return { kind: "unknown", id }
  if (entry.node.revoked_at !== null) return { kind: "revoked", entry }
  return { kind: "active", entry, keyed: entry.public_key !== null }
}

/** Nodes `*` covers right now: registered and not revoked, with or without a key. */
export function activeInventory(inventory: readonly ConnectionNode[]): ConnectionNode[] {
  return inventory.filter((entry) => entry.node.revoked_at === null)
}

/** Current members of a tag (assignments of active nodes only). */
export function tagMembers(tags: TagSnapshot, tag: string, inventory: readonly ConnectionNode[]): ConnectionNode[] {
  const ids = new Set(tags.assignments.filter((assignment) => assignment.tags.includes(tag)).map((a) => a.node_id))
  return activeInventory(inventory).filter((entry) => ids.has(entry.node.id))
}

// --- Validation ---------------------------------------------------------------

/** `field` uses the request-body path (`rules[0].source[1]`), like controller diagnostics. */
export type AccessIssue = { ruleIndex: number | null; field: string; message: string }

/**
 * Checks before saving; the controller validates again and stays the
 * authority. Unknown or revoked nodes and tags missing from the catalog make
 * the saved policy invalid (everything denied), so they block saving here.
 */
export function validateAccess(
  policy: MeshAccessPolicy,
  inventory: readonly ConnectionNode[],
  tags: TagSnapshot
): AccessIssue[] {
  const issues: AccessIssue[] = []
  if (policy.rules.length > ACCESS_LIMITS.rules) {
    issues.push({ ruleIndex: null, field: "rules", message: `最多 ${ACCESS_LIMITS.rules} 条规则。` })
  }
  if (new TextEncoder().encode(JSON.stringify(policy)).length > ACCESS_LIMITS.policyBytes) {
    issues.push({ ruleIndex: null, field: "rules", message: "策略总大小超过 128 KiB 上限。" })
  }
  const catalog = new Set(tags.tags.map((definition) => definition.tag))
  const ids = new Map<string, number>()
  for (const rule of policy.rules) ids.set(rule.id, (ids.get(rule.id) ?? 0) + 1)
  policy.rules.forEach((rule, index) => {
    const add = (field: string, message: string) =>
      issues.push({ ruleIndex: index, field: `rules[${index}]${field ? `.${field}` : ""}`, message })
    if (!RULE_ID_PATTERN.test(rule.id)) add("id", "规则 ID 为 1–64 位字母、数字、点、下划线或连字符。")
    else if ((ids.get(rule.id) ?? 0) > 1) add("id", "规则 ID 必须唯一。")
    for (const side of ["source", "destination"] as const) {
      const list = rule[side]
      for (const issue of selectorListIssues(list)) {
        add(issue.index === null ? side : `${side}[${issue.index}]`, issue.message)
      }
      list.forEach((raw, position) => {
        const selector = parseSelector(raw)
        if (selector.kind === "node") {
          const ref = resolveAccessNode(inventory, selector.id)
          if (ref.kind === "unknown") add(`${side}[${position}]`, "节点不在清单中；引用它会使整个授权无效。")
          if (ref.kind === "revoked") add(`${side}[${position}]`, "节点已撤销；引用它会使整个授权无效。")
        } else if (selector.kind === "tag" && !catalog.has(selector.tag)) {
          add(`${side}[${position}]`, `标签目录中没有 ${selector.tag}；先在标签页创建。`)
        }
      })
    }
  })
  return issues
}

export function issuesForRule(issues: readonly AccessIssue[], index: number): AccessIssue[] {
  return issues.filter((issue) => issue.ruleIndex === index)
}

/** Controller diagnostics carry the rule ID in `rule_id`. */
export function diagnosticsForRule(diagnostics: readonly PolicyDiagnostic[], id: string): PolicyDiagnostic[] {
  return diagnostics.filter((diagnostic) => diagnostic.rule_id === id)
}

// --- Draft vs saved -------------------------------------------------------------

export type RuleChange = "new" | "modified" | "unchanged"

/** Rules are identified by ID; a renamed rule reads as new. */
export function ruleChange(rule: MeshAccessRule, base: MeshAccessPolicy): RuleChange {
  const saved = base.rules.find((candidate) => candidate.id === rule.id)
  if (!saved) return "new"
  return jsonEqual(saved, rule) ? "unchanged" : "modified"
}

export function removedRuleCount(draft: MeshAccessPolicy, base: MeshAccessPolicy): number {
  const ids = new Set(draft.rules.map((rule) => rule.id))
  return base.rules.filter((rule) => !ids.has(rule.id)).length
}

/**
 * An earlier rule that already matches every flow this rule could match in
 * one direction, judged on selector text only: `*` covers anything, and an
 * identical selector set covers itself. Membership is not expanded, so this
 * never claims a shadow the controller would not see.
 */
export function shadowedBy(policy: MeshAccessPolicy, index: number): number | null {
  const rule = policy.rules[index]
  if (!rule) return null
  const covers = (outer: readonly string[], inner: readonly string[]) =>
    outer.includes("*") || inner.every((selector) => outer.includes(selector))
  const directions = (candidate: MeshAccessRule) =>
    candidate.bidirectional
      ? [
          [candidate.source, candidate.destination],
          [candidate.destination, candidate.source],
        ]
      : [[candidate.source, candidate.destination]]
  for (let earlier = 0; earlier < index; earlier += 1) {
    const candidate = policy.rules[earlier]
    if (!candidate) continue
    const shadowed = directions(rule).every(([source = [], destination = []]) =>
      directions(candidate).some(
        ([outerSource = [], outerDestination = []]) => covers(outerSource, source) && covers(outerDestination, destination)
      )
    )
    if (shadowed) return earlier
  }
  return null
}

/** Field paths from validation or diagnostics, for display. */
export function accessFieldLabel(field: string): string {
  const match = /(source|destination)(?:\[(\d+)\])?$/.exec(field)
  if (match) {
    const side = match[1] === "source" ? "来源" : "目标"
    return match[2] === undefined ? side : `${side}第 ${Number(match[2]) + 1} 项`
  }
  if (field.endsWith(".id")) return "规则 ID"
  if (field === "rules") return "规则列表"
  if (field === "default_action") return "默认动作"
  return field
}

// --- Save outcome -------------------------------------------------------------

export type AccessSaveFailure =
  | { status: "conflict"; message: string }
  | { status: "invalid"; message: string }
  | { status: "error"; message: string }

export function accessSaveFailure(
  httpStatus: number | null,
  expectedRevision: number,
  detail: string
): AccessSaveFailure {
  if (httpStatus === 409) {
    return {
      status: "conflict",
      message: `Controller 的配置版本已不是 r${expectedRevision}：节点、标签、路由、连接或授权在你编辑期间被修改。未保存任何内容，草稿仍保留在编辑器中。`,
    }
  }
  if (httpStatus === 422) {
    return {
      status: "invalid",
      message: `Controller 拒绝保存：${detail}。已保存的授权保持不变。`,
    }
  }
  return { status: "error", message: `保存失败：${detail}` }
}
