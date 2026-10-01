"use client"

import type { ConnectionNode, TagSnapshot } from "@meshway/control-client"
import Link from "next/link"
import { useId } from "react"

import { Segmented } from "@/components/common/segmented"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { ALL_SELECTOR_HINT, nodeSelector } from "@/lib/access-selector"
import { activeInventory, tagMembers } from "@/lib/access-model"
import { cn } from "@/lib/utils"

/**
 * Chooses one endpoint list: either `*` alone, or any mix of tags and
 * nodes. Every choice shows what it covers (tag members, node address), so
 * no opaque ID is ever picked blind. Selectors already in the list that are
 * no longer valid stay listed and checked until explicitly removed.
 */
export function SelectorPicker({
  legend,
  value,
  onChange,
  inventory,
  tags,
  disabled,
  invalid,
  testId,
}: {
  legend: string
  value: readonly string[]
  onChange: (next: string[]) => void
  inventory: readonly ConnectionNode[]
  tags: TagSnapshot
  disabled?: boolean
  invalid?: boolean
  testId?: string
}) {
  const id = useId()
  const all = value.includes("*")
  const active = activeInventory(inventory)
  const known = new Set([...tags.tags.map((definition) => definition.tag), ...active.map((entry) => nodeSelector(entry.node.id))])
  const stray = value.filter((selector) => selector !== "*" && !known.has(selector))
  const toggle = (selector: string, checked: boolean) =>
    onChange(checked ? [...value.filter((item) => item !== "*" && item !== selector), selector] : value.filter((item) => item !== selector))

  return (
    <fieldset data-testid={testId} aria-invalid={invalid || undefined} className="flex flex-col gap-2" disabled={disabled}>
      <legend className="mb-1 text-sm font-medium">{legend}</legend>
      <Segmented
        size="sm"
        aria-label={`${legend}范围`}
        value={all ? "all" : "some"}
        onValueChange={(mode) => onChange(mode === "all" ? ["*"] : value.filter((item) => item !== "*"))}
        options={[
          { value: "all", label: "全部节点（*）" },
          { value: "some", label: "选择标签或节点" },
        ]}
        disabled={disabled}
      />
      {all ? (
        <p className="text-xs text-muted-foreground">{ALL_SELECTOR_HINT}</p>
      ) : (
        <div className={cn("flex flex-col gap-3 rounded-lg border p-3", invalid && "border-destructive/50")}>
          <div className="flex flex-col gap-1.5">
            <span className="flex items-center justify-between text-xs font-medium text-muted-foreground">
              标签
              <Link href="/tags" className="font-normal underline underline-offset-2">
                管理标签
              </Link>
            </span>
            {tags.tags.length === 0 ? (
              <p className="text-xs text-muted-foreground">还没有标签。</p>
            ) : (
              tags.tags.map((definition) => {
                const members = tagMembers(tags, definition.tag, inventory)
                const checkboxId = `${id}-${definition.tag}`
                return (
                  <div key={definition.tag} className="flex items-start gap-2">
                    <Checkbox
                      id={checkboxId}
                      checked={value.includes(definition.tag)}
                      onCheckedChange={(checked) => toggle(definition.tag, checked === true)}
                    />
                    <Label htmlFor={checkboxId} className="flex min-w-0 flex-col items-start gap-0.5 font-normal">
                      <span className="text-xs font-medium">{definition.tag.slice(4)}</span>
                      <span className="text-[0.6875rem] text-muted-foreground">
                        {members.length > 0 ? members.map((entry) => entry.node.name).join("、") : "暂无成员"}
                        {definition.description ? ` · ${definition.description}` : ""}
                      </span>
                    </Label>
                  </div>
                )
              })
            )}
          </div>
          <div className="flex flex-col gap-1.5">
            <span className="text-xs font-medium text-muted-foreground">节点</span>
            {active.length === 0 ? (
              <p className="text-xs text-muted-foreground">没有已注册的活动节点。</p>
            ) : (
              active.map((entry) => {
                const selector = nodeSelector(entry.node.id)
                const checkboxId = `${id}-${entry.node.id}`
                return (
                  <div key={entry.node.id} className="flex items-start gap-2">
                    <Checkbox
                      id={checkboxId}
                      checked={value.includes(selector)}
                      onCheckedChange={(checked) => toggle(selector, checked === true)}
                    />
                    <Label htmlFor={checkboxId} className="flex min-w-0 flex-col items-start gap-0.5 font-normal">
                      <span className="text-xs font-medium">{entry.node.name}</span>
                      <span className="font-mono text-[0.6875rem] text-muted-foreground" title={entry.node.id}>
                        {entry.node.mesh_ipv4} · {entry.node.id.slice(0, 8)}
                        {entry.public_key === null ? " · 待公钥（规则有效，数据面暂不执行）" : ""}
                      </span>
                    </Label>
                  </div>
                )
              })
            )}
          </div>
          {stray.length > 0 ? (
            <div className="flex flex-col gap-1.5">
              <span className="text-xs font-medium text-destructive">无效引用（移除后才能保存）</span>
              {stray.map((selector) => {
                const checkboxId = `${id}-stray-${selector}`
                return (
                  <div key={selector} className="flex items-center gap-2">
                    <Checkbox id={checkboxId} checked onCheckedChange={() => toggle(selector, false)} />
                    <Label htmlFor={checkboxId} className="font-mono text-xs font-normal break-all text-destructive">
                      {selector}
                    </Label>
                  </div>
                )
              })}
            </div>
          ) : null}
        </div>
      )}
    </fieldset>
  )
}
