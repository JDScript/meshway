// Endpoint selector strings of the confirmed access v2 wire format:
// "*" | "node:<64 lowercase hex>" | "tag:<name>". Display and editor
// pre-checks only; the generated schema and the controller remain the
// authority and replace these patterns once generated.

export type Selector =
  | { kind: "all" }
  | { kind: "node"; id: string }
  | { kind: "tag"; tag: string; name: string }
  | { kind: "invalid"; raw: string }

export const TAG_SELECTOR = /^tag:[a-z0-9][a-z0-9_-]{0,47}$/
export const NODE_SELECTOR = /^node:[0-9a-f]{64}$/
export const SELECTORS_MAX = 32

export function parseSelector(raw: string): Selector {
  if (raw === "*") return { kind: "all" }
  if (NODE_SELECTOR.test(raw)) return { kind: "node", id: raw.slice(5) }
  if (TAG_SELECTOR.test(raw)) return { kind: "tag", tag: raw, name: raw.slice(4) }
  return { kind: "invalid", raw }
}

export const nodeSelector = (id: string) => `node:${id}`

export type SelectorIssue = { index: number | null; message: string }

/** Pre-checks one source or destination list. */
export function selectorListIssues(list: readonly string[]): SelectorIssue[] {
  const issues: SelectorIssue[] = []
  if (list.length === 0) issues.push({ index: null, message: "至少选择一个端点（或 *）。" })
  if (list.length > SELECTORS_MAX) issues.push({ index: null, message: `最多 ${SELECTORS_MAX} 个端点。` })
  if (list.includes("*") && list.length > 1) {
    issues.push({ index: list.indexOf("*"), message: "* 已代表全部已注册节点，不能与其他端点组合。" })
  }
  const seen = new Set<string>()
  list.forEach((raw, index) => {
    if (seen.has(raw)) issues.push({ index, message: "重复的端点。" })
    seen.add(raw)
    if (parseSelector(raw).kind === "invalid") {
      issues.push({ index, message: "端点必须是 *、node:<64 位节点 ID> 或 tag:<小写名称>。" })
    }
  })
  return issues
}

export const ALL_SELECTOR_LABEL = "全部已注册节点"
export const ALL_SELECTOR_HINT = "* 是当前网络中所有已注册、未撤销的节点，包括之后注册的节点；不包括互联网、未注册设备或子网。"

/**
 * The same rule read in reverse: a bidirectional rule also matches traffic
 * from its destination list to its source list, at the same position.
 */
export function reversed<T extends { source: readonly string[]; destination: readonly string[] }>(
  rule: T
): { source: readonly string[]; destination: readonly string[] } {
  return { source: rule.destination, destination: rule.source }
}
