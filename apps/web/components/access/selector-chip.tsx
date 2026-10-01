import type { ConnectionNode, TagSnapshot } from "@meshway/control-client"
import { AsteriskIcon, ServerIcon, TagIcon } from "lucide-react"

import { parseSelector } from "@/lib/access-selector"
import { activeInventory, resolveAccessNode, tagMembers } from "@/lib/access-model"
import { cn } from "@/lib/utils"

const shortId = (id: string) => id.slice(0, 8)

/** Display of one endpoint selector: `*`, a tag with its member count, or a node by name. */
export function SelectorChip({
  selector,
  inventory,
  tags,
  className,
}: {
  selector: string
  inventory: readonly ConnectionNode[]
  tags: TagSnapshot
  className?: string
}) {
  const parsed = parseSelector(selector)
  const base = "inline-flex h-6 max-w-full min-w-0 items-center gap-1 rounded-md border px-1.5 text-xs"
  switch (parsed.kind) {
    case "all": {
      const count = activeInventory(inventory).length
      return (
        <span className={cn(base, "border-brand/40 bg-brand/5", className)} title="当前网络中所有已注册、未撤销的节点，包括之后注册的节点">
          <AsteriskIcon aria-hidden="true" className="size-3 text-brand" />
          <span className="font-medium">全部节点</span>
          <span className="text-muted-foreground tabular-nums">{count}</span>
        </span>
      )
    }
    case "tag": {
      const known = tags.tags.some((definition) => definition.tag === parsed.tag)
      const members = tagMembers(tags, parsed.tag, inventory)
      return (
        <span
          className={cn(base, known ? "bg-background" : "border-destructive/50 text-destructive", className)}
          title={known ? `成员：${members.map((entry) => entry.node.name).join("、") || "无"}` : "标签目录中没有这个标签"}
        >
          <TagIcon aria-hidden="true" className="size-3 text-muted-foreground" />
          <span className="truncate font-medium">{parsed.name}</span>
          <span className="text-muted-foreground tabular-nums">{known ? members.length : "未知"}</span>
        </span>
      )
    }
    case "node": {
      const ref = resolveAccessNode(inventory, parsed.id)
      const broken = ref.kind !== "active"
      return (
        <span className={cn(base, broken ? "border-destructive/50 text-destructive" : "bg-background", className)} title={parsed.id}>
          <ServerIcon aria-hidden="true" className="size-3 text-muted-foreground" />
          <span className="truncate font-medium">
            {ref.kind === "unknown" ? `未知节点 ${shortId(parsed.id)}` : ref.entry.node.name}
          </span>
          {ref.kind === "revoked" ? <span>（已撤销）</span> : null}
          {ref.kind === "active" && !ref.keyed ? <span className="text-warning">待公钥</span> : null}
        </span>
      )
    }
    case "invalid":
      return (
        <span className={cn(base, "border-destructive/50 font-mono text-destructive", className)}>{parsed.raw || "（空）"}</span>
      )
  }
}

/** A selector list, with each tag's current members spelled out underneath. */
export function SelectorList({
  list,
  inventory,
  tags,
  label,
}: {
  list: readonly string[]
  inventory: readonly ConnectionNode[]
  tags: TagSnapshot
  label: string
}) {
  const tagLines = list.flatMap((selector) => {
    const parsed = parseSelector(selector)
    if (parsed.kind !== "tag" || !tags.tags.some((definition) => definition.tag === parsed.tag)) return []
    const members = tagMembers(tags, parsed.tag, inventory)
    return [{ tag: parsed.name, members: members.map((entry) => entry.node.name) }]
  })
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <ul aria-label={label} className="flex min-w-0 flex-wrap gap-1">
        {list.map((selector, index) => (
          <li key={`${index}-${selector}`} className="min-w-0">
            <SelectorChip selector={selector} inventory={inventory} tags={tags} />
          </li>
        ))}
      </ul>
      {tagLines.length > 0 ? (
        <p className="text-[0.6875rem] text-muted-foreground">
          {tagLines.map((line, index) => (
            <span key={line.tag}>
              {index > 0 ? "；" : ""}
              {line.tag}：{line.members.length > 0 ? line.members.join("、") : "暂无成员"}
            </span>
          ))}
        </p>
      ) : null}
    </div>
  )
}
