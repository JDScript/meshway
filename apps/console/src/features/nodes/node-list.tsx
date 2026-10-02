import type {
  ConnectionRuntimesSnapshot,
  NodeSummary,
} from "@meshway/control-client"
import { SearchIcon } from "lucide-react"
import { useEffect, useMemo, useRef, type KeyboardEvent } from "react"

import { SegmentedControl } from "@/components/chrome/segmented-control"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Skeleton } from "@/components/ui/skeleton"
import { highlightOnGlobe } from "@/features/globe/store"
import { vitalityLabel } from "@/features/overview/labels"
import { vitalityOf } from "@/features/overview/model"
import { VitalityDot } from "@/features/overview/status-dots"
import type { NodeListView } from "@/features/shell/route"
import { NODE_STATES, type NodeStateFilter } from "@/features/shell/url"
import { m } from "@/paraglide/messages.js"

const PLACEHOLDER_ROWS = 5
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

const matches = (node: NodeSummary, needle: string) =>
  needle === "" ||
  [node.name, node.mesh_ipv4, node.mesh_ipv6].some((value) =>
    value.toLowerCase().includes(needle)
  )

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
}: {
  nodes: readonly NodeSummary[]
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
}) {
  const list = useRef<HTMLUListElement>(null)

  const rows = useMemo(
    () =>
      nodes
        .filter((node) => node.revoked_at === null)
        .sort(
          (a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id)
        )
        .map((node) => ({ node, vitality: vitalityOf(node, runtimes, now) })),
    [nodes, runtimes, now]
  )
  const needle = view.q.trim().toLowerCase()
  const shown = rows.filter(
    (row) =>
      (view.state === null || row.vitality === view.state) &&
      matches(row.node, needle)
  )

  // Keep the open row in view, wherever it was opened from.
  useEffect(() => {
    if (!openId) return
    list.current
      ?.querySelector(`[data-node-id="${openId}"]`)
      ?.scrollIntoView({ block: "nearest" })
  }, [openId])

  const onKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return
    event.preventDefault()
    const focused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement.dataset.nodeId
        : undefined
    const index = shown.findIndex((row) => row.node.id === (focused ?? openId))
    const next =
      shown[
        index === -1
          ? 0
          : Math.min(
              shown.length - 1,
              Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1))
            )
      ]
    if (!next) return
    list.current
      ?.querySelector<HTMLButtonElement>(`[data-node-id="${next.node.id}"]`)
      ?.focus()
    if (openId) onOpen(next.node.id)
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-col gap-3 px-4 pt-4 pb-3">
        <div className="flex items-baseline justify-between">
          <h2
            id="node-list-title"
            className="text-base font-semibold tracking-tight"
          >
            {m.nav_nodes()}
          </h2>
          <span className="text-xs text-muted-foreground tabular-nums">
            {loading
              ? "—"
              : shown.length === rows.length
                ? rows.length
                : `${shown.length} / ${rows.length}`}
          </span>
        </div>
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
      </header>

      <ul
        ref={list}
        aria-labelledby="node-list-title"
        onKeyDown={onKeyDown}
        onPointerLeave={() => highlightOnGlobe(null)}
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
      >
        {loading
          ? Array.from({ length: PLACEHOLDER_ROWS }, (_, index) => (
              <li key={index} className="flex h-14 items-center gap-3 px-2">
                <Skeleton className="size-2 rounded-full" />
                <Skeleton className="h-3 w-32 rounded-full" />
              </li>
            ))
          : null}
        {!loading && shown.length === 0 ? (
          <li className="flex h-14 items-center px-2 text-row text-muted-foreground">
            {failed
              ? m.overview_nodes_failed()
              : rows.length === 0
                ? m.nodes_empty()
                : m.nodes_no_match()}
          </li>
        ) : null}
        {shown.map(({ node, vitality }) => (
          <li key={node.id}>
            <button
              type="button"
              data-node-id={node.id}
              aria-current={node.id === openId ? "true" : undefined}
              onClick={() => onOpen(node.id)}
              onPointerEnter={() => highlightOnGlobe(node.id)}
              className="list-row h-14 gap-3 px-2"
            >
              <VitalityDot vitality={vitality} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-row font-medium">
                  {node.name}
                </span>
                <span className="block truncate font-mono text-meta text-muted-foreground">
                  {node.mesh_ipv4}
                </span>
              </span>
              <span className="shrink-0 text-meta text-muted-foreground">
                {vitalityLabel[vitality]()}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}
