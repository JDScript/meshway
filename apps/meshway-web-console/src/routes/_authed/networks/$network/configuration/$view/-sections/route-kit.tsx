import type { RouteAction } from "@meshway/control-client"
import {
  AtSignIcon,
  GlobeIcon,
  ListIcon,
  NetworkIcon,
  RouteIcon,
  ServerIcon,
  SparklesIcon,
  WaypointsIcon,
} from "lucide-react"
import { type ReactNode } from "react"

import { SegmentedControl } from "@/components/chrome/segmented-control"
import {
  ChipSetField,
  type ChipKind,
  type Shown,
} from "@/components/configuration/chips"
import {
  refusedAt,
  RemarkInput,
  RemoveButton,
  type Names,
  type Note,
} from "@/components/configuration/form-kit"
import { pointerOf, type JsonPath } from "@/lib/configuration/jsonc"
import {
  isCountry,
  isNodeSelector,
  matchKind,
  profileKind,
  SET_PREFIX,
  type ProfileChoice,
} from "@/lib/configuration/routing"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { MoveButtons } from "./ordered"

// The values the routing and rule-set forms pick: concrete nodes (via
// nodes, exits), destinations (domain suffixes, prefixes, rule sets),
// profiles and countries; and where a matched flow goes.

/**
 * Concrete nodes. For exits (`exits` given), a node not enabled as an exit
 * is marked: the controller refuses it as an exit.
 */
export function nodeChips(
  names: Names,
  exits: ReadonlySet<string> | null
): ChipKind {
  return {
    options: [...names.nodes].map(([id, hostname]) => ({
      value: `node:${id}`,
      label: hostname,
      detail:
        exits !== null && !exits.has(id)
          ? m.configuration_routing_not_exit()
          : id.slice(0, 8),
      group: m.configuration_selector_nodes(),
    })),
    accepts: isNodeSelector,
    show: (value): Shown => {
      const id = value.slice(5)
      const hostname = isNodeSelector(value) ? names.nodes.get(id) : undefined
      return hostname
        ? {
            icon: ServerIcon,
            label: hostname,
            mono: false,
            known: exits === null || exits.has(id),
          }
        : {
            icon: ServerIcon,
            label: m.configuration_selector_unknown_node({
              id: id.slice(0, 8),
            }),
            mono: true,
            known: false,
          }
    },
    placeholder: m.configuration_routing_node_placeholder(),
  }
}

/** Destinations: domain suffixes and prefixes typed, rule sets (`set:<key>`) picked or typed. */
export function destinationChips(
  ruleSets: readonly string[],
  withSets: boolean
): ChipKind {
  return {
    options: withSets
      ? ruleSets.map((key) => ({
          value: SET_PREFIX + key,
          label: key,
          group: m.configuration_section_ruleSets(),
        }))
      : [],
    accepts: (value) => {
      const kind = matchKind(value)
      if (kind === "set")
        return withSets && ruleSets.includes(value.slice(SET_PREFIX.length))
      // A set's name typed picks the set (offered first), not a domain suffix of that name.
      if (withSets && ruleSets.includes(value)) return false
      return kind !== null
    },
    show: (value): Shown => {
      const found = matchKind(value)
      const kind = found === "set" && !withSets ? null : found
      if (kind === "set") {
        const key = value.slice(SET_PREFIX.length)
        return {
          icon: ListIcon,
          label: key,
          mono: false,
          known: ruleSets.includes(key),
        }
      }
      return {
        icon: kind === "cidr" ? NetworkIcon : AtSignIcon,
        label: value,
        mono: true,
        known: kind !== null,
      }
    },
    placeholder: withSets
      ? m.configuration_routing_destination_placeholder()
      : m.configuration_ruleset_inline_placeholder(),
  }
}

/** A profile's name as people read it. */
export function profileLabel(name: string, names: Names): string {
  switch (profileKind(name)) {
    case "mesh":
      return m.configuration_routing_profile_mesh()
    case "auto":
      return m.configuration_routing_profile_auto()
    case "global": {
      const id = name.slice(7)
      return m.configuration_routing_profile_global({
        node: names.nodes.get(id) ?? id.slice(0, 8),
      })
    }
    case "custom":
      return name
  }
}

const PROFILE_ICON = {
  mesh: WaypointsIcon,
  auto: SparklesIcon,
  global: GlobeIcon,
  custom: RouteIcon,
} as const

/** Profiles among `choices`; one not among them is marked (the controller refuses it). */
export function profileChips(
  choices: readonly ProfileChoice[],
  names: Names
): ChipKind {
  const values = choices.map((choice) => choice.value)
  return {
    options: choices.map((choice) => ({
      value: choice.value,
      label: profileLabel(choice.value, names),
      detail: choice.kind === "custom" ? undefined : choice.value.slice(0, 16),
      group:
        choice.kind === "custom"
          ? m.configuration_routing_profiles()
          : m.configuration_routing_builtin(),
    })),
    accepts: (value) => values.includes(value),
    show: (value): Shown => ({
      icon: PROFILE_ICON[profileKind(value)],
      label: profileLabel(value, names),
      mono: false,
      known: values.includes(value),
    }),
    placeholder: m.configuration_routing_profile_placeholder(),
  }
}

/** Countries and regions, as two uppercase letters. */
export function countryChips(): ChipKind {
  return {
    options: [],
    accepts: isCountry,
    show: (value) => ({
      icon: null,
      label: value,
      mono: true,
      known: isCountry(value),
    }),
    placeholder: m.configuration_routing_country_placeholder(),
  }
}

type ActionType = RouteAction["type"]

const ACTION_LABEL: Record<ActionType, () => string> = {
  exit: () => m.configuration_routing_action_exit(),
  direct: () => m.configuration_routing_action_direct(),
  unmanaged: () => m.configuration_routing_action_unmanaged(),
}

/** Where matched traffic goes: exits in priority order (with nodes before them), the source's own network, or not taken over. */
export function ActionField({
  action,
  path,
  fallback,
  editable,
  names,
  exits,
  notes,
  onChange,
}: {
  action: RouteAction
  path: JsonPath
  /** A profile's fallback: it may also leave traffic unmanaged. */
  fallback: boolean
  editable: boolean
  names: Names
  exits: ReadonlySet<string>
  notes: readonly Note[]
  onChange: (action: RouteAction) => void
}) {
  const types: ActionType[] = fallback
    ? ["exit", "direct", "unmanaged"]
    : ["exit", "direct"]
  const choose = (type: ActionType) => {
    if (type === action.type) return
    onChange(type === "exit" ? { type, nodes: [] } : { type })
  }
  return (
    <span className="inline-flex min-w-0 flex-wrap items-center gap-1.5">
      {editable ? (
        <SegmentedControl
          size="sm"
          label={m.configuration_routing_action()}
          value={action.type}
          options={types.map((type) => ({
            value: type,
            label: ACTION_LABEL[type](),
          }))}
          onChange={choose}
        />
      ) : (
        <span
          className={cn(
            "inline-flex h-6 shrink-0 items-center rounded-full bg-fill-subtle px-2.5 text-meta font-medium"
          )}
        >
          {ACTION_LABEL[action.type]()}
        </span>
      )}
      {action.type === "exit" ? (
        <>
          <ChipSetField
            values={action.nodes}
            kind={nodeChips(names, exits)}
            label={m.configuration_routing_exits()}
            addLabel={m.configuration_routing_exits_add()}
            editable={editable}
            invalid={refusedAt(notes, pointerOf([...path, "nodes"]))}
            onChange={(nodes) => onChange({ ...action, nodes })}
          />
          {editable || (action.via ?? []).length > 0 ? (
            <>
              <span className="text-meta text-muted-foreground">
                {m.configuration_routing_exit_via()}
              </span>
              <ChipSetField
                values={action.via ?? []}
                kind={nodeChips(names, null)}
                label={m.configuration_routing_exit_via_label()}
                addLabel={m.configuration_routing_via_add()}
                editable={editable}
                invalid={refusedAt(notes, pointerOf([...path, "via"]))}
                onChange={(via) => onChange({ ...action, via })}
              />
            </>
          ) : null}
        </>
      ) : null}
    </span>
  )
}

export const replaceAt = <T,>(
  items: readonly T[],
  index: number,
  next: T
): T[] => items.map((item, at) => (at === index ? next : item))

export function Hint({ children }: { children: ReactNode }) {
  return <p className="text-meta text-muted-foreground">{children}</p>
}

/** A rule's end: its remark, moving it, removing it. */
export function RowEnd({
  remark,
  editable,
  index,
  count,
  label,
  onRemark,
  onMove,
  onRemove,
}: {
  remark: string
  editable: boolean
  index: number
  count: number
  label: string
  onRemark: (remark: string) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  return (
    <>
      <RemarkInput value={remark} editable={editable} onChange={onRemark} />
      {editable ? (
        <>
          <MoveButtons
            index={index}
            count={count}
            label={label}
            onMove={onMove}
          />
          <RemoveButton
            label={m.configuration_rule_remove({ index: index + 1 })}
            onRemove={onRemove}
          />
        </>
      ) : null}
    </>
  )
}

export type Common = {
  editable: boolean
  names: Names
  notes: readonly Note[]
  exits: ReadonlySet<string>
  ruleSets: readonly string[]
}
