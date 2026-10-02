import type { NodeSummary, TagDefinition } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { Command } from "cmdk"
import { cn } from "@/lib/utils"
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
import { POPUP_CLEARANCE } from "@/features/shell/layout"
import { m } from "@/paraglide/messages.js"

import { tagsQuery } from "@/features/network/queries"

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

/**
 * A chip that opens a picker, 28px: pointing at it and its open picker tint
 * it (`.live-chip`; a ring would read as a fault); the keyboard rings it.
 */
const LIVE_CHIP =
  "live-chip h-7 outline-none has-focus-visible:ring-2 has-focus-visible:ring-ring/50 focus-visible:ring-2 focus-visible:ring-ring/50"

/** A selector the controller refused, or one that names nothing known. */
const flagged = (known: boolean, invalid: boolean) =>
  cn(
    !known && "border-warning/50",
    invalid && "border-destructive/60 bg-destructive/8"
  )

/** A selector's icon and name, inside a chip. */
function ChipContent({
  selector,
  names,
}: {
  selector: string
  names: SelectorNames
}) {
  const shown = selectorShown(selector, names)
  const Icon = shown.icon
  return (
    <>
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
    </>
  )
}

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
  const known = selectorShown(selector, names).known
  return (
    <span className={cn(CHIP, flagged(known, false), className)}>
      <ChipContent selector={selector} names={names} />
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
 * Its trigger has no look of its own: `className` and `children` make it
 * the chip to replace, or "add". It keeps
 * clear of the screen's edges and the commit bar, and its list shrinks to
 * the room there is. While it shows a node, it also leads to the node.
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
  autoOpen = false,
  onClosed,
  node,
  onOpenNode,
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
  /** Opens as soon as it is shown (a rule just added). */
  autoOpen?: boolean
  onClosed?: () => void
  /** The selector it replaces, when that is a node: offered as a way to the node. */
  node?: string
  onOpenNode?: (nodeId: string) => void
}) {
  const [open, setOpen] = useState(autoOpen && !disabled)
  const { inventory } = useNetwork()
  const all = options(names, kinds, inventory)
  const nodeId = node?.startsWith("node:") ? node.slice(5) : null
  const known = nodeId !== null && names.node(nodeId) !== undefined
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) onClosed?.()
      }}
    >
      <PopoverTrigger
        disabled={disabled}
        aria-label={label}
        aria-invalid={invalid || undefined}
        className={cn(
          "inline-flex min-w-0 items-center disabled:pointer-events-none",
          className
        )}
      >
        {children}
      </PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={6}
        collisionPadding={POPUP_CLEARANCE}
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
          {/* The search, the gaps and the way to a node take the rest of the room. */}
          <Command.List className="max-h-[min(300px,calc(var(--available-height)-6rem))] overflow-y-auto overscroll-contain">
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
                            onClosed?.()
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
        {nodeId !== null && known && onOpenNode ? (
          <button
            type="button"
            onClick={() => {
              setOpen(false)
              onOpenNode(nodeId)
            }}
            className="list-row mt-1 h-9 gap-2 border-t border-divider px-3 text-row text-muted-foreground hover:text-foreground"
          >
            <ArrowUpRightIcon className="size-3.5" />
            {m.policy_selector_open_node()}
          </button>
        ) : null}
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
  invalid = false,
  autoOpen,
  onClosed,
  onOpenNode,
  onChange,
}: {
  value: string
  names: SelectorNames
  kinds: readonly SelectorKind[]
  label: string
  disabled?: boolean
  invalid?: boolean
  autoOpen?: boolean
  onClosed?: () => void
  onOpenNode?: (nodeId: string) => void
  onChange: (selector: string) => void
}) {
  const known = selectorShown(value, names).known
  if (disabled)
    return <SelectorChip selector={value} names={names} className="h-7" />
  return (
    <SelectorPicker
      names={names}
      kinds={kinds}
      chosen={[value]}
      label={label}
      invalid={invalid}
      autoOpen={autoOpen}
      onClosed={onClosed}
      node={value}
      onOpenNode={onOpenNode}
      onPick={onChange}
      className={cn(CHIP, LIVE_CHIP, flagged(known, invalid))}
    >
      <ChipContent selector={value} names={names} />
    </SelectorPicker>
  )
}

/**
 * A set of selectors (a grant's sources or destinations), any of which
 * matches: each chip opens the picker to replace it and has its own remove;
 * one more can be added at the end.
 */
export function SelectorSetField({
  values,
  names,
  kinds,
  label,
  addLabel,
  disabled,
  invalid,
  autoOpen,
  onClosed,
  onOpenNode,
  onChange,
  joiner,
  empty,
  quietAdd = false,
}: {
  values: readonly string[]
  names: SelectorNames
  kinds: readonly SelectorKind[]
  /** Names each chip's picker. */
  label: string
  addLabel: string
  /** Between chips, for a sequence (a route's hops). */
  joiner?: ReactNode
  /** Said when there is none and none can be added. */
  empty?: string
  /** "Add" shows only while its rule row is pointed at (always on touch), for rows with several sets. */
  quietAdd?: boolean
  disabled?: boolean
  invalid?: (index: number) => boolean
  /** Opens the first chip's picker as soon as it is shown (a rule just added). */
  autoOpen?: boolean
  onClosed?: () => void
  onOpenNode?: (nodeId: string) => void
  onChange: (selectors: string[]) => void
}) {
  return (
    <span className="inline-flex min-w-0 flex-wrap items-center gap-1">
      {values.length === 0 && (disabled || empty === undefined) ? (
        <span className="text-row text-muted-foreground">
          {empty ?? m.policy_selector_none()}
        </span>
      ) : null}
      {values.map((selector, index) => {
        const shown = selectorShown(selector, names)
        return (
          <span
            key={`${selector}-${index}`}
            className="inline-flex min-w-0 items-center gap-1"
          >
            {index > 0 ? joiner : null}
            {disabled ? (
              <SelectorChip selector={selector} names={names} className="h-7" />
            ) : (
              // One pill: the name opens the picker, the × inside removes (a 20px circle, 4px in: 14 − 4 = 10).
              <span
                className={cn(
                  CHIP,
                  LIVE_CHIP,
                  "gap-0 px-0",
                  flagged(shown.known, invalid?.(index) ?? false)
                )}
              >
                <SelectorPicker
                  names={names}
                  kinds={kinds}
                  chosen={values}
                  label={label}
                  autoOpen={autoOpen && index === 0}
                  onClosed={onClosed}
                  node={selector}
                  onOpenNode={onOpenNode}
                  onPick={(next) =>
                    onChange(
                      values.map((value, at) => (at === index ? next : value))
                    )
                  }
                  className="h-full min-w-0 gap-1.5 rounded-l-full pr-1 pl-2 outline-none"
                >
                  <ChipContent selector={selector} names={names} />
                </SelectorPicker>
                <button
                  type="button"
                  aria-label={m.policy_selector_remove({ name: shown.label })}
                  onClick={() =>
                    onChange(values.filter((_, at) => at !== index))
                  }
                  className="mr-1 inline-flex size-5 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors outline-none hover:bg-fill-selected hover:text-foreground"
                >
                  <XIcon className="size-3" />
                </button>
              </span>
            )}
          </span>
        )
      })}
      {disabled ? null : (
        <SelectorPicker
          names={names}
          kinds={kinds}
          chosen={values}
          label={addLabel}
          onPick={(selector) => onChange([...values, selector])}
          className={cn(
            "size-7 justify-center rounded-full border border-dashed border-divider text-muted-foreground transition-[color,background-color,opacity] outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-popup-open:bg-fill-selected data-popup-open:text-foreground",
            quietAdd &&
              "opacity-0 group-focus-within/rule:opacity-100 group-hover/rule:opacity-100 data-popup-open:opacity-100 pointer-coarse:opacity-100"
          )}
        >
          <PlusIcon aria-hidden="true" className="size-3" />
        </SelectorPicker>
      )}
    </span>
  )
}
