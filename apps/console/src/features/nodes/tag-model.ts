import type { TagDefinition, TagSnapshot } from "@meshway/control-client"
import { zTagDefinition } from "@meshway/control-client/schemas"

// The tag catalog as it is edited: one list under the network's revision.
// Names follow the generated schema (`tag:` and a lowercase name); a tag
// still assigned to nodes or referenced by policy is refused by the
// controller when deleted.

export const TAG_PREFIX = "tag:"
export const MAX_TAGS = 64

export type TagProblem =
  | { kind: "name"; tag: string }
  | { kind: "description"; tag: string }
  | { kind: "duplicate"; tag: string }
  | { kind: "too_many" }

export function tagProblems(tags: readonly TagDefinition[]): TagProblem[] {
  const problems: TagProblem[] = []
  if (tags.length > MAX_TAGS) problems.push({ kind: "too_many" })
  const seen = new Set<string>()
  for (const definition of tags) {
    if (!zTagDefinition.shape.tag.safeParse(definition.tag).success)
      problems.push({ kind: "name", tag: definition.tag })
    else if (
      !zTagDefinition.shape.description.safeParse(definition.description)
        .success
    )
      problems.push({ kind: "description", tag: definition.tag })
    if (seen.has(definition.tag))
      problems.push({ kind: "duplicate", tag: definition.tag })
    seen.add(definition.tag)
  }
  return problems
}

/** The full name for what was typed after the `tag:` prefix. */
export const tagName = (typed: string) =>
  `${TAG_PREFIX}${typed.trim().replace(/^tag:/, "")}`

export const isTagName = (tag: string) =>
  zTagDefinition.shape.tag.safeParse(tag).success

/** Active nodes holding each tag, as saved. */
export function tagMembers(snapshot: TagSnapshot | undefined) {
  const members = new Map<string, string[]>()
  for (const assignment of snapshot?.assignments ?? [])
    for (const tag of assignment.tags)
      members.set(tag, [...(members.get(tag) ?? []), assignment.node_id])
  return members
}
