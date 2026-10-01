import type { ConnectionNode, MeshAccessPolicy, TagDefinition, TagSnapshot } from "@meshway/control-client"

import { TAG_SELECTOR } from "./access-selector.ts"

// Administrator-owned tag catalog and node bindings. Tags are only names for
// groups of nodes used by access rules; saving the catalog or a node's tags
// moves the global configuration revision like any other change.

export const TAG_LIMITS = { catalog: 64, perNode: 32, description: 256 } as const

export type TagIssue = { index: number | null; field: "tag" | "description" | "tags"; message: string }

/** Pre-checks for the catalog draft; the controller validates again. */
export function validateCatalog(tags: readonly TagDefinition[]): TagIssue[] {
  const issues: TagIssue[] = []
  if (tags.length > TAG_LIMITS.catalog) {
    issues.push({ index: null, field: "tags", message: `最多 ${TAG_LIMITS.catalog} 个标签。` })
  }
  const seen = new Set<string>()
  tags.forEach((definition, index) => {
    if (!TAG_SELECTOR.test(definition.tag)) {
      issues.push({ index, field: "tag", message: "名称为 tag: 加 1–48 位小写字母、数字、下划线或连字符，且以字母或数字开头。" })
    } else if (seen.has(definition.tag)) {
      issues.push({ index, field: "tag", message: "标签重复。" })
    }
    seen.add(definition.tag)
    if ([...definition.description].length > TAG_LIMITS.description) {
      issues.push({ index, field: "description", message: `说明最多 ${TAG_LIMITS.description} 个字符。` })
    }
  })
  return issues
}

/** `prod` or `tag:prod` → `tag:prod`, lower-cased and trimmed. */
export function normalizeTagInput(text: string): string {
  const trimmed = text.trim().toLowerCase()
  return trimmed.startsWith("tag:") ? trimmed : `tag:${trimmed}`
}

export function tagsOfNode(snapshot: TagSnapshot, nodeId: string): string[] {
  return snapshot.assignments.find((assignment) => assignment.node_id === nodeId)?.tags ?? []
}

/** Active member nodes of each catalog tag (revoked nodes are never members). */
export function membership(snapshot: TagSnapshot, inventory: readonly ConnectionNode[]): Map<string, ConnectionNode[]> {
  const result = new Map<string, ConnectionNode[]>(snapshot.tags.map((definition) => [definition.tag, []]))
  for (const entry of inventory) {
    if (entry.node.revoked_at !== null) continue
    for (const tag of tagsOfNode(snapshot, entry.node.id)) result.get(tag)?.push(entry)
  }
  return result
}

/** Why a catalog tag cannot be deleted right now (the controller answers 422 otherwise). */
export function deletionBlocker(
  tag: string,
  snapshot: TagSnapshot,
  access: MeshAccessPolicy | null
): string | null {
  const assigned = snapshot.assignments.filter((assignment) => assignment.tags.includes(tag)).length
  const rules = access?.rules.filter((rule) => rule.source.includes(tag) || rule.destination.includes(tag)) ?? []
  if (assigned > 0 && rules.length > 0) return `已分配给 ${assigned} 个节点，并被 ${rules.length} 条访问规则引用`
  if (assigned > 0) return `已分配给 ${assigned} 个节点`
  if (rules.length > 0) return `被 ${rules.length} 条已保存的访问规则引用`
  return null
}

/** Catalog in the controller's canonical order (sorted by tag). */
export function canonicalCatalog(tags: readonly TagDefinition[]): TagDefinition[] {
  return [...tags].map((definition) => ({ tag: definition.tag, description: definition.description })).sort((a, b) =>
    a.tag < b.tag ? -1 : a.tag > b.tag ? 1 : 0
  )
}

export const catalogKey = (tags: readonly TagDefinition[]) =>
  JSON.stringify(canonicalCatalog(tags).map((definition) => [definition.tag, definition.description]))
