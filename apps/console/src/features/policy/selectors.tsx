import type { NodeSummary, TagDefinition } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { Command } from "cmdk"
import { cn } from "cn"
import {
  ArrowUpRightIcon,
  AsteriskIcon,
  AtSignIcon,
  CheckIcon,
  GlobeIcon,
  NetworkIcon,
  PlusIcon,
  ServerIcon,
  TagIcon,
  XIcon,
  type LucideIcon,
} from "lucide-react"
import { useMemo, useState, type ReactNode } from "react"

import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { useNetwork } from "@/features/network/use-network-view"
import { m } from "@/paraglide/messages.js"

import { tagsQuery } from "./queries"

// Selectors as the policy writes them (`*`, `tag:<name>`, `node:<id>`,
// `autogroup:internet`, `ip:<range>`, `domain:<suffix>`), shown by name
// and picked from the network's real nodes and tags.

export type SelectorNames = {
  node: (id: string) => NodeSummary | undefined
  tags: readonly TagDefinition[]
  /** Nodes carrying each tag. */
  members: ReadonlyMap<string, number>
}

export function useSelectorNames(networkId: string): SelectorNames {
  const { byId } = useNetwork()
  const tags = useQuery(tagsQuery(networkId))
  return useMemo(() => {
    const members = new Map<string, number>()
    for (const assignment of tags.data?.assignments ?? [])
      for (const tag of assignment.tags)
        members.set(tag, (members.get(tag) ?? 0) + 1)
    return {
      node: (id: string) => byId.get(id),
      tags: tags.data?.tags ?? [],
      members,
    }
  }, [byId, tags.data])
}

type Shown = { icon: LucideIcon; label: string; mono: boolean; known: boolean }

export function selectorShown(selector: string, names: SelectorNames): Shown {
  if (selector === "*")
    return {
      icon: AsteriskIcon,
      label: m.policy_selector_all(),
      mono: false,
      known: true,
    }
  if (selector === "autogroup:internet")
    return {
      icon: GlobeIcon,
      label: m.policy_selector_internet(),
      mono: false,
      known: true,
    }
  if (selector.startsWith("node:")) {
    const id = selector.slice(5)
    const node = names.node(id)
    return node
      ? { icon: ServerIcon, label: node.name, mono: false, known: true }
      : {
          icon: ServerIcon,
          label: m.policy_selector_unknown_node({ id: id.slice(0, 8) }),
          mono: false,
          known: false,
        }
  }
  if (selector.startsWith("tag:"))
    return {
      icon: TagIcon,
      label: selector.slice(4),
      mono: false,
      known: names.tags.some((tag) => tag.tag === selector),
    }
  if (selector.startsWith("ip:"))
    return {
      icon: NetworkIcon,
      label: selector.slice(3),
      mono: true,
      known: true,
    }
  if (selector.startsWith("domain:"))
    return {
      icon: AtSignIcon,
      label: selector.slice(7),
      mono: true,
      known: true,
    }
  return { icon: AsteriskIcon, label: selector, mono: true, known: false }
}

/** Node IDs and `node:<id>` in the controller's text, written as node names. */
export function withNodeNames(text: string, names: SelectorNames): string {
  return text.replace(
    /(node:)?\b([0-9a-f]{64})\b/g,
    (whole, _prefix, id: string) => {
      const node = names.node(id)
      return node ? node.name : whole
    }
  )
}

const CHIP =
  "inline-flex h-6 max-w-full min-w-0 items-center gap-1.5 rounded-full border border-divider bg-fill-raised/60 px-2 text-row leading-none"

/** One selector, by name. */
export function SelectorChip({
  selector,
  names,
  className,
}: {
  selector: string
  names: SelectorNames
  className?: string
}) {
  const shown = selectorShown(selector, names)
  const Icon = shown.icon
  return (
    <span
      className={cn(CHIP, !shown.known && "border-warning/50", className)}
      title={selector}
    >
      <Icon
        aria-hidden="true"
        className={cn(
          "size-3 shrink-0",
          shown.known ? "text-muted-foreground" : "text-warning"
        )}
      />
      <span className={cn("truncate", shown.mono && "font-mono text-meta")}>
        {shown.label}
      </span>
    </span>
  )
}

/** Several selectors (a grant's sources), any of which matches. */
export function SelectorGroup({
  selectors,
  names,
}: {
  selectors: readonly string[]
  names: SelectorNames
}) {
  return (
    <span className="inline-flex min-w-0 flex-wrap items-center gap-1">
      {selectors.length === 0 ? (
        <span className="text-row text-muted-foreground">
          {m.policy_selector_none()}
        </span>
      ) : (
        selectors.map((selector, index) => (
          <SelectorChip
            key={`${selector}-${index}`}
            selector={selector}
            names={names}
          />
        ))
      )}
    </span>
  )
}

export type SelectorKind = "all" | "tag" | "node"

type Option = {
  selector: string
  label: string
  detail: string
  group: SelectorKind
}

function options(
  names: SelectorNames,
  kinds: readonly SelectorKind[],
  inventory: readonly NodeSummary[]
): Option[] {
  const out: Option[] = []
  if (kinds.includes("all"))
    out.push({
      selector: "*",
      label: m.policy_selector_all(),
      detail: m.policy_selector_count({ count: inventory.length }),
      group: "all",
    })
  if (kinds.includes("tag"))
    for (const tag of names.tags)
      out.push({
        selector: tag.tag,
        label: tag.tag.slice(4),
        detail: m.policy_selector_count({
          count: names.members.get(tag.tag) ?? 0,
        }),
        group: "tag",
      })
  if (kinds.includes("node"))
    for (const node of inventory)
      out.push({
        selector: `node:${node.id}`,
        label: node.name,
        detail: node.mesh_ipv4,
        group: "node",
      })
  return out
}

const GROUP_LABEL: Record<SelectorKind, () => string> = {
  all: () => m.policy_selector_group_all(),
  tag: () => m.policy_selector_group_tags(),
  node: () => m.policy_selector_group_nodes(),
}

/**
 * Picks a selector from the network's tags and nodes, searchable by name.
 * `trigger` is the button that opens it (a chip to replace, or "add").
 */
export function SelectorPicker({
  names,
  kinds,
  chosen,
  label,
  disabled,
  invalid,
  onPick,
  children,
  className,
}: {
  names: SelectorNames
  kinds: readonly SelectorKind[]
  /** Already chosen: marked, and not picked twice. */
  chosen: readonly string[]
  /** The accessible name of the trigger. */
  label: string
  disabled?: boolean
  invalid?: boolean
  onPick: (selector: string) => void
  children: ReactNode
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const { inventory } = useNetwork()
  const all = options(names, kinds, inventory)
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        disabled={disabled}
        aria-label={label}
        aria-invalid={invalid || undefined}
        className={cn(
          "inline-flex min-w-0 items-center rounded-full transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none aria-invalid:ring-2 aria-invalid:ring-destructive/60 data-popup-open:ring-2 data-popup-open:ring-ring/60",
          className
        )}
      >
        {children}
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="glass w-72 gap-0 rounded-surface p-2"
      >
        <Command
          label={label}
          className="flex flex-col"
          filter={(value, search, keywords) =>
            [value, ...(keywords ?? [])].some((word) =>
              word.toLowerCase().includes(search.toLowerCase())
            )
              ? 1
              : 0
          }
        >
          <Command.Input
            autoFocus
            placeholder={m.policy_selector_search()}
            className="mb-1 h-9 w-full rounded-row bg-field px-3 text-row outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring"
          />
          <Command.List className="max-h-[min(50svh,300px)] overflow-y-auto">
            <Command.Empty className="px-3 py-3 text-row text-muted-foreground">
              {m.policy_selector_empty()}
            </Command.Empty>
            {(["all", "tag", "node"] as const)
              .filter((group) => kinds.includes(group))
              .map((group) => {
                const items = all.filter((option) => option.group === group)
                if (items.length === 0) return null
                return (
                  <Command.Group
                    key={group}
                    heading={GROUP_LABEL[group]()}
                    className="[&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:text-meta [&_[cmdk-group-heading]]:text-muted-foreground"
                  >
                    {items.map((option) => {
                      const taken = chosen.includes(option.selector)
                      return (
                        <Command.Item
                          key={option.selector}
                          value={option.selector}
                          keywords={[option.label, option.detail]}
                          disabled={taken}
                          onSelect={() => {
                            onPick(option.selector)
                            setOpen(false)
                          }}
                          className="flex h-9 cursor-pointer items-center gap-2.5 rounded-row px-3 text-row outline-none data-[disabled=true]:cursor-default data-[selected=true]:bg-fill-hover"
                        >
                          <span className="min-w-0 flex-1 truncate">
                            {option.label}
                          </span>
                          <span className="shrink-0 font-mono text-meta text-muted-foreground tabular-nums">
                            {option.detail}
                          </span>
                          <CheckIcon
                            aria-hidden="true"
                            className={cn(
                              "size-3.5 shrink-0",
                              taken ? "opacity-100" : "opacity-0"
                            )}
                          />
                        </Command.Item>
                      )
                    })}
                  </Command.Group>
                )
              })}
          </Command.List>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

/** One selector, chosen with the picker (a connection's end, a route's source). */
export function SelectorField({
  value,
  names,
  kinds,
  label,
  disabled,
  invalid,
  onChange,
}: {
  value: string
  names: SelectorNames
  kinds: readonly SelectorKind[]
  label: string
  disabled?: boolean
  invalid?: boolean
  onChange: (selector: string) => void
}) {
  return (
    <SelectorPicker
      names={names}
      kinds={kinds}
      chosen={[value]}
      label={label}
      disabled={disabled}
      invalid={invalid}
      onPick={onChange}
    >
      <SelectorChip
        selector={value}
        names={names}
        className={cn("h-7", !disabled && "hover:bg-fill-hover")}
      />
    </SelectorPicker>
  )
}

/** A set of selectors (a grant's sources or destinations): chips to remove, one to add. */
export function SelectorSetField({
  values,
  names,
  kinds,
  label,
  disabled,
  invalid,
  onChange,
}: {
  values: readonly string[]
  names: SelectorNames
  kinds: readonly SelectorKind[]
  label: string
  disabled?: boolean
  invalid?: (index: number) => boolean
  onChange: (selectors: string[]) => void
}) {
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-1">
      {values.map((selector, index) => (
        <span
          key={`${selector}-${index}`}
          className={cn(
            "inline-flex min-w-0 items-center rounded-full",
            invalid?.(index) && "ring-2 ring-destructive/60"
          )}
        >
          <SelectorChip
            selector={selector}
            names={names}
            className={cn(
              "h-7",
              !disabled && "rounded-r-none border-r-0 pr-1.5"
            )}
          />
          {disabled ? null : (
            <button
              type="button"
              aria-label={m.policy_selector_remove({
                name: selectorShown(selector, names).label,
              })}
              onClick={() => onChange(values.filter((_, at) => at !== index))}
              className="inline-flex h-7 items-center rounded-r-full border border-l-0 border-divider bg-fill-raised/60 pr-2 pl-1 text-muted-foreground transition-colors outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <XIcon className="size-3" />
            </button>
          )}
        </span>
      ))}
      {disabled ? null : (
        <SelectorPicker
          names={names}
          kinds={kinds}
          chosen={values}
          label={label}
          onPick={(selector) => onChange([...values, selector])}
        >
          <span className="inline-flex h-7 items-center gap-1 rounded-full border border-dashed border-divider px-2.5 text-row text-muted-foreground transition-colors hover:bg-fill-hover hover:text-foreground">
            <PlusIcon className="size-3" />
            {m.policy_selector_add()}
          </span>
        </SelectorPicker>
      )}
    </span>
  )
}

/** A node named in a rule, as a link to its workspace. */
export function NodeLink({
  selector,
  names,
  onOpenNode,
}: {
  selector: string
  names: SelectorNames
  onOpenNode: (nodeId: string) => void
}) {
  if (!selector.startsWith("node:")) return null
  const id = selector.slice(5)
  const node = names.node(id)
  if (!node) return null
  return (
    <button
      type="button"
      onClick={() => onOpenNode(id)}
      className="inline-flex h-7 items-center gap-1 rounded-full px-2 text-meta text-muted-foreground transition-colors outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
    >
      {node.name}
      <ArrowUpRightIcon className="size-3" />
    </button>
  )
}
