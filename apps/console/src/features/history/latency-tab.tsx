import { useQuery } from "@tanstack/react-query"
import { useMemo, useState } from "react"

import { EmptyLine } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { nodesQuery } from "@/features/nodes/queries"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { millis, percent, span } from "./format"
import { LatencyChart } from "./latency-chart"
import { historyFailureText } from "./queries"
import { bucketSeries, unansweredRate, type Series } from "./series"
import { bucketsOf, itemCount, useHistory } from "./use-history"
import { historyWindow, type WindowChoice } from "./window"
import { CoverageNotes, WindowChoices, WindowRange } from "./window-control"

type Direction = "out" | "in"

const kindLabel: Record<Series["kind"], () => string> = {
  neighbor: () => m.history_kind_neighbor(),
  path: () => m.history_kind_path(),
  connect: () => m.history_kind_connect(),
  controller: () => m.history_kind_controller(),
}

/**
 * A node's recorded probes in a window the person chooses: those it sent,
 * or those sent to it, one series per peer, path, link, transport and kind.
 * It reads what the controller recorded; it never starts a measurement.
 */
export function LatencyTab({
  networkId,
  nodeId,
}: {
  networkId: string
  nodeId: string
}) {
  const [direction, setDirection] = useState<Direction>("out")
  const [window, setWindow] = useState(() => historyWindow("1h", Date.now()))
  const choose = (choice: WindowChoice) =>
    setWindow(historyWindow(choice, Date.now()))

  const nodes = useQuery(nodesQuery(networkId))
  const nameOf = useMemo(() => {
    const names = new Map(
      (nodes.data?.items ?? []).map((node) => [node.id, node.hostname])
    )
    return (id: string) => names.get(id) ?? id.slice(0, 8)
  }, [nodes.data])

  const filter = useMemo(
    () =>
      direction === "out"
        ? {
            mode: "buckets" as const,
            kinds: ["probe_completed" as const],
            sourceNodeId: nodeId,
          }
        : {
            mode: "buckets" as const,
            kinds: ["probe_completed" as const],
            peerNodeId: nodeId,
          },
    [direction, nodeId]
  )
  const history = useHistory(networkId, window, filter)
  const series = useMemo(
    () => bucketSeries(bucketsOf(history.pages), window),
    [history.pages, window]
  )
  const { query } = history

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <SegmentedControl
            size="sm"
            label={m.history_direction()}
            value={direction}
            options={[
              { value: "out", label: m.history_direction_out() },
              { value: "in", label: m.history_direction_in() },
            ]}
            onChange={setDirection}
          />
          <WindowChoices
            value={window.choice}
            onChange={(c) => c && choose(c)}
          />
        </div>
        <WindowRange
          window={window}
          busy={query.isFetching}
          onRefresh={() => choose(window.choice)}
        />
        <div className="text-meta text-muted-foreground">
          {m.history_bucket({ value: span(window.bucketSeconds) })}
        </div>
        <CoverageNotes
          coverage={history.coverage}
          window={window}
          nameOf={nameOf}
        />
      </div>

      {query.data === undefined ? (
        query.isError ? (
          <EmptyLine>
            {failureText(query.error, historyFailureText) ?? m.history_failed()}
          </EmptyLine>
        ) : (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-6 w-48 rounded-full" />
            <Skeleton className="h-36 w-full rounded-tile" />
          </div>
        )
      ) : series.length === 0 && history.complete ? (
        <EmptyLine>
          {direction === "out" ? m.history_none_out() : m.history_none_in()}
        </EmptyLine>
      ) : (
        <div className="flex flex-col gap-8">
          {series.map((item) => (
            <SeriesView
              key={item.key}
              series={item}
              direction={direction}
              window={window}
              nameOf={nameOf}
            />
          ))}
        </div>
      )}

      {query.isError && query.data !== undefined ? (
        <EmptyLine>
          {failureText(query.error, historyFailureText) ?? m.history_failed()}
        </EmptyLine>
      ) : null}
      {history.waiting ? (
        <div className="flex items-center gap-3 text-meta text-muted-foreground">
          <span>
            {m.history_partial({ count: String(itemCount(history.pages)) })}
          </span>
          <Button variant="outline" size="sm" onClick={history.more}>
            {m.history_more()}
          </Button>
        </div>
      ) : null}
    </div>
  )
}

function SeriesView({
  series,
  direction,
  window,
  nameOf,
}: {
  series: Series
  direction: Direction
  window: ReturnType<typeof historyWindow>
  nameOf: (id: string) => string
}) {
  const other = direction === "out" ? series.peerNodeId : series.sourceNodeId
  const via = series.path.slice(1, -1)
  const rate = unansweredRate(series.totals)
  const title =
    direction === "out" ? `→ ${nameOf(other)}` : `${nameOf(other)} →`
  return (
    <section className="flex flex-col gap-2">
      <div className="flex min-w-0 flex-col">
        <h3 className="truncate text-row font-medium">{title}</h3>
        <span className="truncate text-meta text-muted-foreground">
          {[
            kindLabel[series.kind](),
            series.transport,
            via.length
              ? m.history_via({ nodes: via.map(nameOf).join(" → ") })
              : null,
            series.linkId ? m.history_link({ id: series.linkId }) : null,
            series.instances > 1
              ? m.history_instances({ count: String(series.instances) })
              : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
        <Tile label={m.history_mean()}>
          {series.totals.meanUs !== null
            ? millis(series.totals.meanUs)
            : m.history_no_success()}
        </Tile>
        <Tile label={m.history_spread()}>
          {series.totals.minUs !== null && series.totals.maxUs !== null
            ? `${millis(series.totals.minUs)} – ${millis(series.totals.maxUs)}`
            : "—"}
        </Tile>
        <Tile label={m.history_samples()}>{series.totals.samples}</Tile>
        <Tile
          label={m.history_unanswered()}
          tone={rate !== null && rate > 0 ? "warning" : undefined}
        >
          {rate !== null ? percent(rate) : "—"}
        </Tile>
      </div>
      <div className="text-meta text-muted-foreground tabular-nums">
        {m.history_counts({
          success: String(series.totals.success),
          timeout: String(series.totals.timeout),
          failed: String(series.totals.failed),
        })}
      </div>
      <div className="rounded-tile bg-fill-subtle px-3.5 pt-3 pb-2">
        <LatencyChart
          series={series}
          window={window}
          label={m.history_chart({ peer: nameOf(other) })}
        />
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-meta text-muted-foreground">
        <Legend swatch="line">{m.history_legend_mean()}</Legend>
        <Legend swatch="warning">{m.history_legend_unanswered()}</Legend>
        <span>{m.history_legend_unmeasured()}</span>
      </div>
    </section>
  )
}

function Legend({
  swatch,
  children,
}: {
  swatch: "line" | "warning"
  children: string
}) {
  return (
    <span className="flex items-center gap-1.5">
      <span
        aria-hidden="true"
        className={
          swatch === "line"
            ? "h-0.5 w-3 rounded-full bg-[var(--signal)]"
            : "size-2 rounded-[2px] bg-warning"
        }
      />
      {children}
    </span>
  )
}
