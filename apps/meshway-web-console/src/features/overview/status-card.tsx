import type { AggregatedLink, StateView } from "@meshway/control-client"
import { ChevronDownIcon } from "lucide-react"
import { useState } from "react"

import { StatusDot } from "@/components/chrome/status-dot"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { failureText } from "@/lib/failure"
import { dateTime, relativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"
import { highlightOnGlobe } from "@/state/globe"

import { Group } from "./group"
import { HistoryLayer, type HistoryLayerProps } from "./history-layer"
import {
  evidenceLabel,
  freshnessColor,
  freshnessLabel,
  lookColor,
  phaseLabel,
  unlocatedReason,
} from "./labels"
import {
  countBy,
  EVIDENCE_ORDER,
  FRESHNESS_ORDER,
  linkKey,
  linkLook,
  PHASE_ORDER,
  type MapNode,
} from "./model"

/**
 * The overview's corner: the State's time and how it is kept current, the
 * nodes by freshness and the links by phase and evidence, the nodes the
 * controller did not locate (with its reason) and the current links, which
 * are drawn only between located ends. Every number is counted from the
 * State as read.
 */
export function StatusCard({
  state,
  stateError,
  nodes,
  located,
  live,
  nameOf,
  onOpenNode,
  history,
  mobile,
}: {
  state: StateView | undefined
  stateError: unknown
  nodes: readonly MapNode[]
  located: ReadonlySet<string>
  /** The event stream keeps the counts current (else they are polled). */
  live: boolean
  nameOf: (id: string) => string
  onOpenNode: (nodeId: string) => void
  history: HistoryLayerProps
  mobile: boolean
}) {
  const now = useNow(5_000)
  const [open, setOpen] = useState(!mobile)
  const links = state?.links ?? []
  const unlocated = nodes.filter((node) => node.unlocated !== null)
  const undrawn = links.filter(
    (link) => !located.has(link.a) || !located.has(link.b)
  )

  const header = (
    <div className="flex h-(--bar-height) shrink-0 items-center gap-2 pr-2 pl-4">
      <h2 className="text-row font-medium">{m.overview_status()}</h2>
      <span className="min-w-0 flex-1 truncate text-meta text-muted-foreground tabular-nums">
        {state ? (
          <span title={dateTime(state.asOf) ?? undefined}>
            {live ? m.overview_live() : m.overview_polling()} ·{" "}
            {m.overview_as_of({
              time: relativeTime(state.asOf, now) ?? "—",
            })}
          </span>
        ) : null}
      </span>
      {mobile ? (
        <button
          type="button"
          aria-expanded={open}
          aria-label={open ? m.overview_collapse() : m.overview_expand()}
          onClick={() => setOpen((value) => !value)}
          className="flex h-8 items-center gap-1.5 rounded-full px-2.5 text-meta text-muted-foreground outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring"
        >
          {state && !open
            ? m.overview_summary({
                nodes: String(nodes.length),
                links: String(links.length),
              })
            : null}
          <ChevronDownIcon
            className={cn(
              "size-4 transition-transform",
              open ? "" : "rotate-180"
            )}
          />
        </button>
      ) : null}
    </div>
  )

  if (!state)
    return (
      <>
        {header}
        <div className="px-4 pb-4 text-row text-muted-foreground">
          {stateError ? (
            (failureText(stateError) ?? m.overview_state_failed())
          ) : (
            <span
              className="flex flex-col gap-2"
              aria-label={m.overview_state_loading()}
            >
              <Skeleton className="h-3 w-40 rounded-full" />
              <Skeleton className="h-3 w-28 rounded-full" />
            </span>
          )}
        </div>
      </>
    )

  return (
    <>
      {header}
      {open ? (
        <div className="scroll-edge flex min-h-0 flex-col gap-4 overflow-y-auto px-4 pb-4">
          <dl className="grid grid-cols-[auto_1fr] items-baseline gap-x-4 gap-y-2 text-row">
            <dt className="text-muted-foreground">{m.overview_nodes()}</dt>
            <dd className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
              <span className="font-medium tabular-nums">{nodes.length}</span>
              {countBy(
                nodes.map((node) => node.freshness),
                FRESHNESS_ORDER
              ).map(({ value, count }) => (
                <span
                  key={value}
                  className="flex items-center gap-1.5 text-meta text-muted-foreground"
                >
                  <StatusDot
                    size="sm"
                    color={freshnessColor(value)}
                    shape={value === "current" ? "live" : "hollow"}
                  />
                  {freshnessLabel[value]()}
                  <span className="text-foreground tabular-nums">{count}</span>
                </span>
              ))}
            </dd>
            <dt className="text-muted-foreground">{m.overview_links()}</dt>
            <dd className="flex min-w-0 flex-col gap-1">
              <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <span className="font-medium tabular-nums">{links.length}</span>
                <Counts
                  items={countBy(
                    links.map((link) => link.phase),
                    PHASE_ORDER
                  ).map(({ value, count }) => [phaseLabel[value](), count])}
                />
              </span>
              <span className="flex flex-wrap gap-x-3 gap-y-1">
                <Counts
                  items={countBy(
                    links.map((link) => link.evidence),
                    EVIDENCE_ORDER
                  ).map(({ value, count }) => [evidenceLabel[value](), count])}
                />
              </span>
            </dd>
          </dl>

          {nodes.length === 0 ? (
            <p className="text-row text-muted-foreground">
              {m.overview_no_nodes()}
            </p>
          ) : null}

          {unlocated.length ? (
            <Group title={m.overview_unlocated()} count={unlocated.length}>
              <ul className="flex flex-col gap-0.5">
                {unlocated.map((node) => (
                  <li key={node.id}>
                    <button
                      type="button"
                      onClick={() => onOpenNode(node.id)}
                      className="list-row group/row flex h-12 w-full items-center gap-3 px-2.5 text-left"
                    >
                      <StatusDot
                        color={freshnessColor(node.freshness)}
                        shape={node.freshness === "current" ? "live" : "hollow"}
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-row font-medium">
                          {node.name}
                        </span>
                        <span className="block truncate text-meta text-muted-foreground">
                          {unlocatedReason(node)}
                        </span>
                      </span>
                      <span className="shrink-0 text-meta text-muted-foreground">
                        {freshnessLabel[node.freshness]()}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </Group>
          ) : null}

          <Group
            title={m.overview_current_links()}
            count={links.length}
            note={
              undrawn.length
                ? m.overview_links_undrawn({ count: String(undrawn.length) })
                : undefined
            }
          >
            {links.length === 0 ? (
              <p className="px-2.5 text-row text-muted-foreground">
                {m.overview_no_links()}
              </p>
            ) : (
              <ul className="flex flex-col gap-0.5">
                {links.map((link) => (
                  <LinkRow
                    key={linkKey(link)}
                    link={link}
                    drawn={located.has(link.a) && located.has(link.b)}
                    nameOf={nameOf}
                    now={now}
                  />
                ))}
              </ul>
            )}
          </Group>

          <HistoryLayer {...history} />
        </div>
      ) : null}
    </>
  )
}

function Counts({ items }: { items: [string, number][] }) {
  return items.map(([label, count]) => (
    <span key={label} className="text-meta text-muted-foreground">
      {label} <span className="text-foreground tabular-nums">{count}</span>
    </span>
  ))
}

function LinkRow({
  link,
  drawn,
  nameOf,
  now,
}: {
  link: AggregatedLink
  drawn: boolean
  nameOf: (id: string) => string
  now: number
}) {
  const use =
    link.inUse === true
      ? m.overview_link_in_use()
      : link.inUse === false
        ? m.overview_link_not_in_use()
        : m.overview_link_use_unknown()
  return (
    <li
      className="flex min-h-12 items-center gap-3 rounded-row px-2.5 py-1.5"
      onPointerEnter={() => highlightOnGlobe(link.a)}
      onPointerLeave={() => highlightOnGlobe(null)}
    >
      <span
        aria-hidden="true"
        className="h-0.5 w-3 shrink-0 rounded-full"
        style={{ background: lookColor[linkLook(link)] }}
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-row font-medium">
          {nameOf(link.a)} ↔ {nameOf(link.b)}
        </span>
        <span className="block truncate text-meta text-muted-foreground">
          {[
            link.transport,
            phaseLabel[link.phase](),
            evidenceLabel[link.evidence](),
          ].join(" · ")}
        </span>
        <span className="block truncate text-meta text-muted-foreground">
          {[
            use,
            m.overview_link_initiator({ node: nameOf(link.initiatorNodeId) }),
            link.since
              ? m.overview_link_since({
                  time: relativeTime(link.since, now) ?? "—",
                })
              : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </span>
      {drawn ? null : (
        <span className="shrink-0 text-meta text-muted-foreground">
          {m.overview_link_unlocated_end()}
        </span>
      )}
    </li>
  )
}
