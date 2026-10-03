import type { NodeSummary } from "@meshway/control-client"
import { useMemo } from "react"
import {
  CartesianGrid,
  Line,
  LineChart,
  ReferenceLine,
  XAxis,
  YAxis,
} from "recharts"

import { RefList, RefRow, RefStateHeading } from "@/components/chrome/ref-row"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Tile } from "@/components/chrome/tile"
import {
  ChartContainer,
  ChartTooltip,
  type ChartConfig,
} from "@/components/ui/chart"
import { Skeleton } from "@/components/ui/skeleton"
import { lookLabel } from "@/features/overview/labels"
import { lookOf } from "@/features/overview/model"
import { LookDot } from "@/features/overview/status-dots"
import { nodeAdjacencies, otherEnd } from "@/features/topology/topology"
import { clockTime, dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { latencyFailureLabel, transportLabel } from "./labels"
import {
  breakAfter,
  buildHistory,
  msText,
  type ChartPoint,
  type History,
} from "./latency-model"
import { channelRtt, nodeName, RttPair, type NodeContext } from "./node-facts"
import {
  LATENCY_SPANS,
  type LatencySpan,
  type LinkDirection,
  type NodeDetail,
} from "./url"
import { useLatencyHistory } from "./use-latency-history"

/**
 * The node's channels with their current evidence; one of them (the one in
 * the URL, else the first) shows its directed RTT history below.
 */
export function NodeLinksTab({
  networkId,
  node,
  context,
  detail,
  onDetail,
  onOpenNode,
}: {
  networkId: string
  node: NodeSummary
  context: NodeContext
  detail: NodeDetail
  onDetail: (detail: NodeDetail) => void
  onOpenNode: (nodeId: string) => void
}) {
  const adjacencies = context.observation
    ? nodeAdjacencies(context.observation, node.id)
    : null
  const peers = adjacencies?.map((adjacency) => otherEnd(adjacency, node.id))
  const selected =
    detail.link !== null && peers?.includes(detail.link)
      ? detail.link
      : (peers?.[0] ?? null)

  return (
    <div className="flex flex-col gap-8">
      <Section
        title={m.node_links()}
        count={adjacencies ? adjacencies.length : null}
        action={
          adjacencies && adjacencies.length > 0 ? (
            <RefStateHeading>{m.node_rtt_heading()}</RefStateHeading>
          ) : null
        }
      >
        {adjacencies === null ? (
          <Skeleton className="h-12 w-full rounded-tile" />
        ) : adjacencies.length === 0 ? (
          <EmptyLine>{m.node_links_none()}</EmptyLine>
        ) : (
          <RefList>
            {adjacencies.map((adjacency) => {
              const peerId = otherEnd(adjacency, node.id)
              const look = lookOf(adjacency.status)
              const channel = adjacency.channels[0]
              return (
                <RefRow
                  key={adjacency.key}
                  lead={<LookDot look={look} />}
                  title={nodeName(context, peerId)}
                  subtitle={
                    channel
                      ? `${transportLabel[channel.transport]()} · ${lookLabel[look]()}`
                      : lookLabel[look]()
                  }
                  state={<RttPair rtt={channelRtt(channel, node.id)} />}
                  current={peerId === selected}
                  onSelect={() => onDetail({ ...detail, link: peerId })}
                  onOpen={() => onOpenNode(peerId)}
                  openLabel={m.node_open_peer({
                    name: nodeName(context, peerId),
                  })}
                />
              )
            })}
          </RefList>
        )}
      </Section>

      {selected ? (
        <LinkHistory
          key={`${selected}/${detail.dir}/${detail.span}`}
          networkId={networkId}
          node={node}
          peer={selected}
          context={context}
          dir={detail.dir}
          span={detail.span}
          onDir={(dir) => onDetail({ ...detail, link: selected, dir })}
          onSpan={(span) => onDetail({ ...detail, link: selected, span })}
        />
      ) : null}
    </div>
  )
}

const chartConfig = {
  rtt: { label: "RTT", color: "var(--chart-1)" },
} satisfies ChartConfig

const spanLabel: Record<LatencySpan, () => string> = {
  "1h": () => m.node_span_1h(),
  "6h": () => m.node_span_6h(),
  "24h": () => m.node_span_24h(),
}

function LinkHistory({
  networkId,
  node,
  peer,
  context,
  dir,
  span,
  onDir,
  onSpan,
}: {
  networkId: string
  node: NodeSummary
  peer: string
  context: NodeContext
  dir: LinkDirection
  span: LatencySpan
  onDir: (dir: LinkDirection) => void
  onSpan: (span: LatencySpan) => void
}) {
  const source = dir === "out" ? node.id : peer
  const target = dir === "out" ? peer : node.id
  const state = useLatencyHistory({ networkId, source, peer: target, span })
  const history = useMemo(
    () =>
      state.range
        ? buildHistory(
            state.records.values(),
            state.range,
            breakAfter(state.expectedIntervalSeconds)
          )
        : null,
    [state]
  )
  const trimmed =
    state.range !== null &&
    state.coverage.trimmedBefore !== null &&
    state.coverage.trimmedBefore > state.range.from

  return (
    <Section
      title={m.node_latency_history()}
      action={
        <SegmentedControl<LatencySpan>
          label={m.node_latency_span()}
          value={span}
          options={LATENCY_SPANS.map((value) => ({
            value,
            label: spanLabel[value](),
          }))}
          onChange={onSpan}
        />
      }
    >
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <SegmentedControl<LinkDirection>
            label={m.node_latency_direction()}
            value={dir}
            options={[
              {
                value: "out",
                label: `${nodeName(context, node.id)} → ${nodeName(context, peer)}`,
              },
              {
                value: "in",
                label: `${nodeName(context, peer)} → ${nodeName(context, node.id)}`,
              },
            ]}
            onChange={onDir}
          />
          <span className="flex-1" />
          <span className="text-meta text-muted-foreground">
            {m.node_latency_source({ node: nodeName(context, source) })}
          </span>
        </div>

        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          <Tile label={m.node_latency_median()}>
            <span className="font-mono tabular-nums">
              {history?.stats.median != null
                ? `${msText(history.stats.median)} ms`
                : "—"}
            </span>
          </Tile>
          <Tile label={m.node_latency_range()}>
            <span className="font-mono tabular-nums">
              {history?.stats.min != null && history.stats.max != null
                ? `${msText(history.stats.min)}–${msText(history.stats.max)} ms`
                : "—"}
            </span>
          </Tile>
          <Tile label={m.node_latency_samples()}>
            <span className="font-mono tabular-nums">
              {history ? String(history.stats.success) : "—"}
            </span>
          </Tile>
          <Tile
            label={m.node_latency_misses()}
            tone={
              history && history.stats.timeout + history.stats.failed > 0
                ? "warning"
                : undefined
            }
          >
            <span className="font-mono tabular-nums">
              {history
                ? m.node_latency_misses_value({
                    timeout: String(history.stats.timeout),
                    failed: String(history.stats.failed),
                  })
                : "—"}
            </span>
          </Tile>
        </div>

        {history && state.range ? (
          <Chart history={history} range={state.range} context={context} />
        ) : state.failed ? (
          <div className="flex h-53 items-center justify-center rounded-tile bg-fill-subtle text-row text-muted-foreground">
            {m.node_latency_unreadable()}
          </div>
        ) : (
          <Skeleton className="h-53 w-full rounded-tile" />
        )}

        <div className="flex h-5 items-center gap-3 truncate px-1 text-meta text-muted-foreground">
          <span className="truncate">{m.node_latency_kind()}</span>
          <span className="flex-1" />
          {trimmed ? (
            <span className="shrink-0 text-warning">
              {m.node_latency_trimmed()}
            </span>
          ) : null}
          {state.partial ? (
            <span className="shrink-0 text-warning">
              {m.node_latency_partial()}
            </span>
          ) : null}
          {state.readAt !== null ? (
            <span
              className={
                state.failed ? "shrink-0 text-warning" : "shrink-0 tabular-nums"
              }
            >
              {state.failed
                ? m.node_latency_failed({ time: clockTime(state.readAt) })
                : m.node_latency_read({ time: clockTime(state.readAt) })}
            </span>
          ) : null}
        </div>
      </div>
    </Section>
  )
}

const Y_AXIS = 44
const TICKS = 6
const RIGHT = 12

function Chart({
  history,
  range,
  context,
}: {
  history: History
  range: { from: number; until: number }
  context: NodeContext
}) {
  const byTime = useMemo(
    () =>
      new Map(
        history.points
          .filter((point) => point.rtt !== null)
          .map((point) => [point.at, point])
      ),
    [history]
  )
  const position = (at: number) =>
    `${((at - range.from) / (range.until - range.from)) * 100}%`
  const empty = history.points.length === 0
  const ticks = Array.from(
    { length: TICKS },
    (_, index) =>
      range.from + ((range.until - range.from) * (index + 0.5)) / TICKS
  )

  return (
    <div className="relative rounded-tile bg-fill-subtle pt-3">
      <ChartContainer
        config={chartConfig}
        className="aspect-auto h-44 w-full"
        role="img"
        aria-label={m.node_latency_history()}
      >
        <LineChart
          data={history.points}
          margin={{ top: 4, right: RIGHT, bottom: 0, left: 0 }}
        >
          <CartesianGrid vertical={false} strokeDasharray="2 4" />
          <XAxis
            dataKey="at"
            type="number"
            domain={[range.from, range.until]}
            ticks={ticks}
            tickFormatter={(at: number) => clockTime(at, { seconds: false })}
            tickLine={false}
            axisLine={false}
            minTickGap={48}
          />
          <YAxis
            width={Y_AXIS}
            tickLine={false}
            axisLine={false}
            allowDecimals={false}
            domain={[0, "auto"]}
            hide={empty}
          />
          {history.changes.map((at) => (
            <ReferenceLine
              key={at}
              x={at}
              stroke="var(--muted-foreground)"
              strokeDasharray="3 3"
              strokeOpacity={0.6}
            />
          ))}
          <ChartTooltip
            cursor={{ stroke: "var(--divider)" }}
            isAnimationActive={false}
            content={({ active, label }) => {
              const point =
                typeof label === "number" ? byTime.get(label) : undefined
              return active && point ? (
                <PointTip point={point} context={context} />
              ) : null
            }}
          />
          <Line
            dataKey="rtt"
            type="linear"
            stroke="var(--color-rtt)"
            strokeWidth={1.75}
            dot={history.points.length <= 40 ? { r: 2 } : false}
            activeDot={{ r: 3 }}
            connectNulls={false}
            isAnimationActive={false}
          />
        </LineChart>
      </ChartContainer>
      {empty ? (
        <div className="pointer-events-none absolute inset-x-0 top-0 bottom-9 flex items-center justify-center text-row text-muted-foreground">
          {m.node_latency_none()}
        </div>
      ) : null}
      <div
        className="relative h-6"
        style={{ marginLeft: Y_AXIS, marginRight: RIGHT }}
        aria-label={m.node_latency_misses()}
      >
        {history.events.map((event, index) => (
          <span
            key={`${event.at}/${index}`}
            title={`${dateTime(event.at) ?? ""} · ${
              event.outcome === "timeout"
                ? m.node_latency_timeout()
                : event.reason
                  ? latencyFailureLabel[event.reason]()
                  : m.node_latency_failure()
            }`}
            className="absolute top-1.5 h-3 w-0.5 -translate-x-1/2 rounded-full"
            style={{
              left: position(event.at),
              background:
                event.outcome === "timeout"
                  ? "var(--warning)"
                  : "var(--destructive)",
            }}
          />
        ))}
      </div>
    </div>
  )
}

function PointTip({
  point,
  context,
}: {
  point: ChartPoint
  context: NodeContext
}) {
  return (
    <div className="glass rounded-tile px-3 py-2 text-meta">
      <div className="font-mono text-row font-medium tabular-nums">
        {point.rtt !== null ? `${msText(point.rtt)} ms` : "—"}
      </div>
      <div className="text-muted-foreground">{clockTime(point.at)}</div>
      {point.carrier ? (
        <div className="mt-1 text-muted-foreground">
          {transportLabel[point.carrier.transport]()} ·{" "}
          <span className="font-mono">{point.carrier.listenerId}</span> ·{" "}
          {m.node_latency_initiator({
            node: nodeName(context, point.carrier.initiatorNodeId),
          })}
        </div>
      ) : null}
    </div>
  )
}
