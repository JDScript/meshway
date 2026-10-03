import type {
  ConnectionRuntimesSnapshot,
  NodeSummary,
} from "@meshway/control-client"
import { SearchIcon } from "lucide-react"
import { useEffect, useMemo, useRef, type ReactNode } from "react"

import {
  ListNote,
  ListPlaceholders,
  ListRow,
  PanelList,
  useListKeys,
} from "@/components/chrome/list"
import { PanelHeader } from "@/components/chrome/panel-header"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { highlightOnGlobe } from "@/features/globe/store"
import { vitalityLabel } from "@/features/overview/labels"
import { vitalityOf } from "@/features/overview/model"
import { VitalityDot } from "@/features/overview/status-dots"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { NODE_STATES, type NodeListView, type NodeStateFilter } from "./url"

const ALL = "all"
type Filter = NodeStateFilter | typeof ALL
/** Typing settles this long before it is written to the address. */
const SEARCH_SETTLE_MS = 150

const filters = (): { value: Filter; label: string }[] => [
  { value: ALL, label: m.nodes_filter_all() },
  ...NODE_STATES.map((state) => ({
    value: state,
    label: vitalityLabel[state](),
  })),
]

const matches = (node: NodeSummary, tags: readonly string[], needle: string) =>
  needle === "" ||
  [node.name, node.mesh_ipv4, node.mesh_ipv6, ...tags].some((value) =>
    value.toLowerCase().includes(needle)
  )

/** How many tags a row shows before it counts the rest. */
const SHOWN_TAGS = 2

/**
 * The list's search field, bound to the URL without ever setting the field
 * while someone types: the field is uncontrolled, nothing is written during
 * an IME composition (pinyin and the like), and a URL change made elsewhere
 * (back / forward) is copied in only if it is not the one this field wrote.
 */
function SearchField({
  value,
  onSettle,
}: {
  value: string
  onSettle: (value: string) => void
}) {
  const field = useRef<HTMLInputElement>(null)
  const composing = useRef(false)
  const written = useRef(value)
  const timer = useRef<number | undefined>(undefined)

  useEffect(() => {
    if (value === written.current) return
    written.current = value
    if (field.current && !composing.current) field.current.value = value
  }, [value])
  useEffect(() => () => window.clearTimeout(timer.current), [])

  const settle = () => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      const next = field.current?.value ?? ""
      if (composing.current || next === written.current) return
      written.current = next
      onSettle(next)
    }, SEARCH_SETTLE_MS)
  }

  return (
    <InputGroup className="h-9 rounded-full border-transparent bg-fill-subtle dark:bg-fill-subtle">
      <InputGroupAddon className="pl-3">
        <SearchIcon />
      </InputGroupAddon>
      <InputGroupInput
        ref={field}
        type="search"
        name="node-search"
        autoComplete="off"
        placeholder={m.nodes_search()}
        aria-label={m.nodes_search()}
        defaultValue={value}
        onChange={() => {
          if (!composing.current) settle()
        }}
        onCompositionStart={() => {
          composing.current = true
        }}
        onCompositionEnd={() => {
          composing.current = false
          settle()
        }}
      />
    </InputGroup>
  )
}

/**
 * Every active node, as the page panel of the nodes page. The search and
 * the state filter live in the URL. Pointing at a row highlights the node on
 * the globe; choosing one opens its workspace; the arrow keys walk the rows
 * (and, while a workspace is open, carry it along).
 */
export function NodeList({
  nodes,
  runtimes,
  now,
  loading,
  failed,
  view,
  openId,
  onView,
  onOpen,
  switcher,
  action,
  tags,
}: {
  nodes: readonly NodeSummary[]
  /** Each node's tag names (without `tag:`); unread tags show none. */
  tags: ReadonlyMap<string, readonly string[]>
  runtimes: ConnectionRuntimesSnapshot | null
  now: number
  /** No inventory read has finished yet. */
  loading: boolean
  /** The inventory could not be read and nothing is cached. */
  failed: boolean
  view: NodeListView
  /** The node whose workspace is open. */
  openId: string | null
  onView: (view: NodeListView) => void
  onOpen: (nodeId: string) => void
  /** Nodes or tags. */
  switcher: ReactNode
  /** Adding a node, for those who can. */
  action: ReactNode
}) {
  const rows = useMemo(
    () =>
      nodes
        .filter((node) => node.revoked_at === null)
        .sort(
          (a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id)
        )
        .map((node) => ({
          node,
          vitality: vitalityOf(node, runtimes, now),
          tags: tags.get(node.id) ?? NO_TAGS,
        })),
    [nodes, runtimes, now, tags]
  )
  const needle = view.q.trim().toLowerCase()
  const shown = rows.filter(
    (row) =>
      (view.state === null || row.vitality === view.state) &&
      matches(row.node, row.tags, needle)
  )

  const { ref: list, onKeyDown } = useListKeys({
    ids: shown.map((row) => row.node.id),
    openId,
    onOpen,
  })

  // Keep the open row in view, wherever it was opened from.
  useEffect(() => {
    if (!openId) return
    list.current
      ?.querySelector(`[data-row-id="${openId}"]`)
      ?.scrollIntoView({ block: "nearest" })
  }, [list, openId])

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader
        page="nodes"
        title={m.nav_nodes()}
        count={
          loading
            ? null
            : shown.length === rows.length
              ? rows.length
              : `${shown.length} / ${rows.length}`
        }
        action={action}
      >
        {switcher}
        <SearchField value={view.q} onSettle={(q) => onView({ ...view, q })} />
        <SegmentedControl<Filter>
          label={m.nodes_filter()}
          value={view.state ?? ALL}
          options={filters()}
          onChange={(value) =>
            onView({ ...view, state: value === ALL ? null : value })
          }
          className="self-start"
        />
      </PanelHeader>

      <PanelList
        ref={list}
        aria-labelledby={surfaceTitleId("nodes", "panel")}
        onKeyDown={onKeyDown}
        onPointerLeave={() => highlightOnGlobe(null)}
      >
        {loading ? <ListPlaceholders rows={5} /> : null}
        {!loading && shown.length === 0 ? (
          <ListNote>
            {failed
              ? m.overview_nodes_failed()
              : rows.length === 0
                ? m.nodes_empty()
                : m.nodes_no_match()}
          </ListNote>
        ) : null}
        {shown.map(({ node, vitality, tags: nodeTags }) => (
          <ListRow
            key={node.id}
            rowId={node.id}
            current={node.id === openId}
            onClick={() => onOpen(node.id)}
            onPointerEnter={() => highlightOnGlobe(node.id)}
            lead={<VitalityDot vitality={vitality} />}
            title={<span className="truncate">{node.name}</span>}
            subtitle={
              <>
                <span className="shrink-0 font-mono">{node.mesh_ipv4}</span>
                <RowTags tags={nodeTags} />
              </>
            }
            trail={vitalityLabel[vitality]()}
          />
        ))}
      </PanelList>
    </div>
  )
}

const NO_TAGS: readonly string[] = []

/** A row's tags after its address: the first few as pills, then how many more (all of them named for assistive tech). */
function RowTags({ tags }: { tags: readonly string[] }) {
  if (tags.length === 0) return null
  const more = tags.length - SHOWN_TAGS
  return (
    <span
      aria-label={m.nodes_row_tags({ tags: tags.join(", ") })}
      className="flex min-w-0 items-center gap-1"
    >
      {tags.slice(0, SHOWN_TAGS).map((tag) => (
        <span
          key={tag}
          aria-hidden="true"
          className="h-4 min-w-0 truncate rounded-full bg-fill-selected px-1.5 leading-4 group-aria-[current=true]/row:bg-fill-raised"
        >
          {tag}
        </span>
      ))}
      {more > 0 ? (
        <span aria-hidden="true" className="shrink-0 tabular-nums">
          +{more}
        </span>
      ) : null}
    </span>
  )
}
