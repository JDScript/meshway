import type { ConfigurationView } from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { useInfiniteQuery } from "@tanstack/react-query"
import {
  AsteriskIcon,
  AtSignIcon,
  CircleAlertIcon,
  GlobeIcon,
  NetworkIcon,
  PlusIcon,
  ServerIcon,
  TagIcon,
  Trash2Icon,
  UserIcon,
  UsersIcon,
  type LucideIcon,
} from "lucide-react"
import { useMemo, type ReactNode } from "react"

import { membersQuery } from "@/api/queries/members"
import { FieldInput } from "@/components/chrome/form"
import type { Said, Severity } from "@/lib/configuration/diagnostics"
import {
  within,
  type LocalCode,
  type LocalProblem,
} from "@/lib/configuration/form-problems"
import { pointerSegments } from "@/lib/configuration/jsonc"
import {
  cidrFamily,
  isDomain,
  isEmail,
  isNamed,
  isProtocol,
  selectorKind,
  type Family,
  type SelectorKind,
} from "@/lib/configuration/selectors"
import { m } from "@/paraglide/messages.js"

import type { ChipKind, ChipOption, SetRefusal, Shown } from "./chips"

// The pieces the section forms share: what the network has to pick from,
// the problems placed by JSON Pointer, and a rule's row. A rule is one row
// that is its own editor: its values open pickers, its remark is edited in
// place and it is removed at its end; nothing expands, so nothing moves.

export type Names = {
  /** Node IDs to hostnames, from the configuration. */
  nodes: ReadonlyMap<string, string>
  /** Member emails, when this account may read the members; null otherwise. */
  users: readonly string[] | null
  groups: readonly string[]
  tags: readonly string[]
}

/** What selectors can name: the configuration's nodes, groups and tags, and the members when readable. */
export function useNames(
  networkId: string,
  view: ConfigurationView | null,
  groups: Readonly<Record<string, unknown>>,
  tags: Readonly<Record<string, unknown>>,
  readMembers: boolean
): Names {
  const members = useInfiniteQuery({
    ...membersQuery(networkId),
    enabled: readMembers,
  })
  return useMemo(() => {
    const nodes = new Map<string, string>()
    for (const [id, node] of Object.entries(view?.nodes ?? {}))
      nodes.set(id, node.hostname)
    return {
      nodes,
      users: members.data
        ? members.data.pages.flatMap((page) =>
            page.items.map((member) => member.email)
          )
        : null,
      groups: Object.keys(groups),
      tags: Object.keys(tags),
    }
  }, [view, members.data, groups, tags])
}

const ICON: Record<SelectorKind, LucideIcon> = {
  all: AsteriskIcon,
  user: UserIcon,
  group: UsersIcon,
  tag: TagIcon,
  node: ServerIcon,
  internet: GlobeIcon,
  domain: AtSignIcon,
  cidr: NetworkIcon,
}

function showSelector(value: string, names: Names, external: boolean): Shown {
  const kind = selectorKind(value, external)
  if (kind === null)
    return { icon: null, label: value, mono: true, known: false }
  const icon = ICON[kind]
  switch (kind) {
    case "all":
      return {
        icon,
        label: m.configuration_selector_all(),
        mono: false,
        known: true,
      }
    case "internet":
      return {
        icon,
        label: m.configuration_selector_internet(),
        mono: false,
        known: true,
      }
    case "node": {
      const id = value.slice(5)
      const hostname = names.nodes.get(id)
      return hostname
        ? { icon, label: hostname, mono: false, known: true }
        : {
            icon,
            label: m.configuration_selector_unknown_node({
              id: id.slice(0, 8),
            }),
            mono: true,
            known: false,
          }
    }
    case "group":
      return {
        icon,
        label: value.slice(6),
        mono: false,
        known: names.groups.includes(value),
      }
    case "tag":
      return {
        icon,
        label: value.slice(4),
        mono: false,
        known: names.tags.includes(value),
      }
    case "user":
      return {
        icon,
        label: value,
        mono: false,
        known: names.users === null || names.users.includes(value),
      }
    case "domain":
      return { icon, label: value.slice(7), mono: true, known: true }
    case "cidr":
      return { icon, label: value, mono: true, known: true }
  }
}

function memberOptions(names: Names): ChipOption[] {
  const out: ChipOption[] = []
  for (const group of names.groups)
    out.push({
      value: group,
      label: group.slice(6),
      group: m.configuration_selector_groups(),
    })
  for (const user of names.users ?? [])
    out.push({
      value: user,
      label: user,
      group: m.configuration_selector_users(),
    })
  return out
}

/** Selectors for access sources and destinations (`external`) and topology ends. */
export function selectorChips(names: Names, external: boolean): ChipKind {
  const options: ChipOption[] = [
    {
      value: "*",
      label: m.configuration_selector_all(),
      group: m.configuration_selector_network(),
    },
  ]
  if (external)
    options.push({
      value: "autogroup:internet",
      label: m.configuration_selector_internet(),
      detail: "autogroup:internet",
      group: m.configuration_selector_network(),
    })
  for (const tag of names.tags)
    options.push({
      value: tag,
      label: tag.slice(4),
      group: m.configuration_selector_tags(),
    })
  options.push(...memberOptions(names))
  for (const [id, hostname] of names.nodes)
    options.push({
      value: `node:${id}`,
      label: hostname,
      detail: id.slice(0, 8),
      group: m.configuration_selector_nodes(),
    })
  return {
    options,
    accepts: (value) => selectorKind(value, external) !== null,
    show: (value) => showSelector(value, names, external),
    placeholder: external
      ? m.configuration_selector_placeholder_dst()
      : m.configuration_selector_placeholder(),
  }
}

/** A group's members: emails. */
export function userChips(names: Names): ChipKind {
  return {
    options: (names.users ?? []).map((user) => ({
      value: user,
      label: user,
      group: m.configuration_selector_users(),
    })),
    accepts: isEmail,
    show: (value) => ({
      icon: UserIcon,
      label: value,
      mono: false,
      known: !isEmail(value)
        ? false
        : names.users === null || names.users.includes(value),
    }),
    placeholder: m.configuration_email_placeholder(),
  }
}

/** A tag's owners: emails and groups. */
export function ownerChips(names: Names): ChipKind {
  return {
    options: memberOptions(names),
    accepts: (value) => isEmail(value) || isNamed(value, "group:"),
    show: (value) =>
      isNamed(value, "group:")
        ? {
            icon: UsersIcon,
            label: value.slice(6),
            mono: false,
            known: names.groups.includes(value),
          }
        : {
            icon: UserIcon,
            label: value,
            mono: false,
            known:
              isEmail(value) &&
              (names.users === null || names.users.includes(value)),
          },
    placeholder: m.configuration_owner_placeholder(),
  }
}

const PROTOCOL_OPTIONS = ["*", "tcp:*", "udp:*", "icmp:*"]

/** An access rule's protocols and ports. */
export function protocolChips(): ChipKind {
  return {
    options: PROTOCOL_OPTIONS.map((value) => ({
      value,
      label: value === "*" ? m.configuration_protocol_all() : value,
      detail: value === "*" ? "*" : undefined,
      group: m.configuration_protocol_group(),
    })),
    accepts: isProtocol,
    show: (value) => ({
      icon: null,
      label: value === "*" ? m.configuration_protocol_all() : value,
      mono: value !== "*",
      known: isProtocol(value),
    }),
    placeholder: m.configuration_protocol_placeholder(),
  }
}

/** Prefixes of one family (address pools). */
export function prefixChips(family: Family): ChipKind {
  return {
    options: [],
    accepts: (value) => cidrFamily(value) === family,
    show: (value) => ({
      icon: null,
      label: value,
      mono: true,
      known: cidrFamily(value) === family,
    }),
    placeholder:
      family === "ipv4"
        ? m.configuration_pool_ipv4_placeholder()
        : m.configuration_pool_ipv6_placeholder(),
  }
}

/** DNS suffixes (search domains). */
export function domainChips(): ChipKind {
  return {
    options: [],
    accepts: isDomain,
    show: (value) => ({
      icon: null,
      label: value,
      mono: true,
      known: isDomain(value),
    }),
    placeholder: m.configuration_dns_suffix_placeholder(),
  }
}

/** Keys of the DNS servers. */
export function resolverChips(keys: readonly string[]): ChipKind {
  return {
    options: keys.map((key) => ({
      value: key,
      label: key,
      group: m.configuration_dns_servers(),
    })),
    accepts: (value) => keys.includes(value),
    show: (value) => ({
      icon: ServerIcon,
      label: value,
      mono: false,
      known: keys.includes(value),
    }),
    placeholder: m.configuration_dns_resolver_placeholder(),
  }
}

const LOCAL: Record<LocalCode, () => string> = {
  empty: () => m.configuration_local_empty(),
  selector: () => m.configuration_local_selector(),
  protocol: () => m.configuration_local_protocol(),
  bidirectional: () => m.configuration_local_bidirectional(),
  member: () => m.configuration_local_member(),
  owner: () => m.configuration_local_owner(),
  domain: () => m.configuration_local_domain(),
  resolver: () => m.configuration_local_resolver(),
  server: () => m.configuration_local_server(),
  default: () => m.configuration_local_default(),
  cidr: () => m.configuration_local_cidr(),
  node: () => m.configuration_local_node(),
  match: () => m.configuration_local_match(),
  destination: () => m.configuration_local_destination(),
  exits: () => m.configuration_local_exits(),
  country: () => m.configuration_local_country(),
  profileName: () => m.configuration_local_profile_name(),
  url: () => m.configuration_local_url(),
  duration: () => m.configuration_local_duration(),
  key: () => m.configuration_local_key(),
  dialer: () => m.configuration_local_dialer(),
  hostname: () => m.configuration_local_hostname(),
  hostname_taken: () => m.configuration_local_hostname_taken(),
  tags: () => m.configuration_local_tags(),
  tag: () => m.configuration_local_tag(),
  address: () => m.configuration_local_address(),
  address_pool: () => m.configuration_local_address_pool(),
  address_taken: () => m.configuration_local_address_taken(),
}

export type Note = {
  path: string
  message: string
  /** Found here, or said by the controller about the draft as it is (or, stale, as it was). */
  origin: "local" | "controller" | "stale"
  /** A warning is about a value accepted as written: it refuses nothing. */
  severity: Severity
}

/** The field a pointer is on or in (`/access/1/src/0`), by the name its form gives it; null for a rule or entry as a whole. */
function fieldOf(path: string): string | null {
  // A rule's field is its third segment (`access/1/src`), a pool its second.
  const [section, entry, rulePart] = pointerSegments(path) ?? []
  if (section === "access")
    return rulePart === "src"
      ? m.configuration_access_src()
      : rulePart === "dst"
        ? m.configuration_access_dst()
        : rulePart === "ip"
          ? m.configuration_access_ip()
          : null
  if (section === "topology")
    return rulePart === "a"
      ? m.configuration_topology_a()
      : rulePart === "b"
        ? m.configuration_topology_b()
        : rulePart === "transports"
          ? m.configuration_topology_transports()
          : null
  if (section === "addressPools")
    return entry === "ipv4"
      ? m.configuration_pool_ipv4()
      : entry === "ipv6"
        ? m.configuration_pool_ipv6()
        : null
  return null
}

const worded = (path: string, message: string) => {
  const field = fieldOf(path)
  return field === null
    ? message
    : m.configuration_note_field({ field, message })
}

/** The problems found here, then what the controller said (`said`), as notes. */
export function notesFrom(
  local: readonly LocalProblem[],
  said: readonly Said[],
  stale: boolean
): Note[] {
  return [
    ...local.map((problem) => ({
      path: problem.path,
      message: worded(problem.path, LOCAL[problem.code]()),
      origin: "local" as const,
      severity: "error" as const,
    })),
    ...said.map((entry) => ({
      path: entry.path,
      message: worded(entry.path, entry.message),
      origin: stale ? ("stale" as const) : ("controller" as const),
      severity: entry.severity,
    })),
  ]
}

/** The notes at `pointer` or inside it. */
export const notesWithin = (notes: readonly Note[], pointer: string) =>
  notes.filter((note) => within(note.path, pointer))

/** A field is refused when a current note is on it or on one of its values (`/access/1/src/0` refuses `/access/1/src`). */
export const refusedAt = (notes: readonly Note[], pointer: string) =>
  notes.some((note) => refuses(note) && within(note.path, pointer))

/** A name is refused by a note on it, not by one on what it names (a profile's rules). */
export const refusedName = (notes: readonly Note[], pointer: string) =>
  notes.some((note) => refuses(note) && note.path === pointer)

/** A current error, not a warning or one said before the latest edits. */
const refuses = (note: Note) =>
  note.origin !== "stale" && note.severity === "error"

const NUMBER = /^(0|[1-9][0-9]*)$/

/**
 * How the set of values at `pointer` is refused (`/access/1/src/2` refuses
 * value 2). `parts`: the arrays its chips join, in order, with their
 * lengths (a route's match: domain suffixes, prefixes, rule sets).
 */
export function setRefusal(
  notes: readonly Note[],
  pointer: string,
  parts?: readonly (readonly [string, number])[]
): SetRefusal {
  const outer = pointerSegments(pointer)?.length ?? 0
  const values = new Set<number>()
  for (const note of notes) {
    if (!refuses(note) || !within(note.path, pointer)) continue
    const rest = (pointerSegments(note.path) ?? []).slice(outer)
    if (parts === undefined) {
      if (rest[0] !== undefined && NUMBER.test(rest[0]))
        values.add(Number(rest[0]))
      continue
    }
    let offset = 0
    for (const [name, count] of parts) {
      if (rest[0] === name) {
        const index =
          rest[1] !== undefined && NUMBER.test(rest[1]) ? Number(rest[1]) : null
        if (index !== null && index < count) values.add(offset + index)
        break
      }
      offset += count
    }
  }
  return { set: refusedAt(notes, pointer), values }
}

/** The notes on one rule or field, as lines under it. */
export function Notes({
  notes,
  className,
}: {
  notes: readonly Note[]
  className?: string
}) {
  if (notes.length === 0) return null
  return (
    <ul className={cn("flex flex-col gap-1", className)}>
      {notes.map((note, index) => (
        <li
          key={`${note.path}-${index}`}
          className="flex items-start gap-2 text-meta"
        >
          <CircleAlertIcon
            aria-hidden="true"
            className={cn(
              "mt-0.5 size-3 shrink-0",
              note.origin === "stale"
                ? "text-muted-foreground"
                : note.severity === "warning"
                  ? "text-warning"
                  : "text-destructive"
            )}
          />
          <span className="min-w-0 break-words text-muted-foreground">
            {note.origin === "stale"
              ? m.configuration_note_stale({ message: note.message })
              : note.message}
          </span>
        </li>
      ))}
    </ul>
  )
}

/** One rule's row: the rule itself, then its end (the action, the remark, removing), then its notes. */
export function RuleRow({
  label,
  children,
  end,
  detail,
  notes,
}: {
  label: string
  children: ReactNode
  end: ReactNode
  /** A further line of the rule, under its first. */
  detail?: ReactNode
  notes: readonly Note[]
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className="rule-row group/rule flex flex-col gap-1.5 rounded-surface p-2"
    >
      <div className="flex min-h-7 flex-wrap items-center gap-x-3 gap-y-2">
        {/* The rule keeps at least this much of the line; short of it, the end moves to its own line. */}
        <div className="flex min-w-0 flex-[1_1_18rem] flex-wrap items-center gap-1.5">
          {children}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-1">{end}</div>
      </div>
      {detail}
      <Notes notes={notes} className="px-1.5 pb-0.5" />
    </div>
  )
}

const REMARK_SLOT = "w-40 shrink-0 text-meta text-muted-foreground"

/** A rule's remark at its row's end: quiet text until pointed at, edited in place. */
export function RemarkInput({
  value,
  editable,
  onChange,
}: {
  value: string
  editable: boolean
  onChange: (value: string) => void
}) {
  if (!editable)
    return (
      <span
        className={cn(REMARK_SLOT, "truncate px-2.5 leading-7")}
        title={value}
      >
        {value}
      </span>
    )
  return (
    <FieldInput
      aria-label={m.configuration_rule_remark()}
      name="remark"
      placeholder={m.configuration_rule_remark_placeholder()}
      autoComplete="off"
      className={cn(
        REMARK_SLOT,
        "h-7 rounded-full bg-transparent px-2.5 text-ellipsis hover:bg-fill-hover focus-visible:bg-field focus-visible:text-foreground md:text-meta dark:bg-transparent dark:hover:bg-fill-hover dark:focus-visible:bg-field"
      )}
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  )
}

/** Removes a rule or an entry; quiet until its row is pointed at (always shown on touch). */
export function RemoveButton({
  label,
  onRemove,
}: {
  label: string
  onRemove: () => void
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      aria-label={label}
      onClick={onRemove}
      className="rounded-full text-muted-foreground opacity-0 transition-opacity group-focus-within/rule:opacity-100 group-hover/rule:opacity-100 hover:text-destructive focus-visible:opacity-100 pointer-coarse:opacity-100"
    >
      <Trash2Icon />
    </Button>
  )
}

/** Adds a rule or an entry, under the list. */
export function AddButton({
  label,
  onAdd,
}: {
  label: string
  onAdd: () => void
}) {
  return (
    <button
      type="button"
      onClick={onAdd}
      className="flex h-11 w-full items-center justify-center gap-1.5 rounded-surface border border-dashed border-divider text-row text-muted-foreground transition-colors outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
    >
      <PlusIcon className="size-3.5" />
      {label}
    </button>
  )
}

/** A list with nothing in it, on its own line. */
export function NoneLine({ children }: { children: ReactNode }) {
  return (
    <p className="flex h-11 items-center rounded-surface bg-fill-subtle px-4 text-row text-muted-foreground">
      {children}
    </p>
  )
}
