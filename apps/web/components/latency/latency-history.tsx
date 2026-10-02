"use client"

import { ArrowRightIcon, Maximize2Icon, RefreshCwIcon } from "lucide-react"
import { useMemo, useState } from "react"

import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { Segmented } from "@/components/common/segmented"
import { LatencyChart, SEGMENT_COLORS, segmentLabel } from "@/components/latency/latency-chart"
import { useLatencyHistory, type LatencyHistoryState } from "@/components/latency/use-latency-history"
import { useNetworkId } from "@/components/network/network-workspace"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import {
  breakAfter,
  buildHistory,
  DEFAULT_WINDOW,
  failureReasonLabels,
  formatMs,
  LATENCY_WINDOWS,
  type LatencyHistory as History,
  type LatencyWindow,
} from "@/lib/latency-history-model"
import { formatClock, formatDateTime } from "@/lib/time"

/** Rows in the accessible record list of the expanded panel. */
const LIST_LIMIT = 200

/**
 * The selected link's directed RTT history. Always historical evidence of
 * authenticated-neighbor probes, kept apart from the link's current state.
 */
export function LatencyHistory({
  nodeIds,
  names,
  defaultSource,
}: {
  nodeIds: [string, string]
  names: Map<string, string>
  defaultSource: string
}) {
  const networkId = useNetworkId()
  const [a, b] = nodeIds
  const [source, setSource] = useState(defaultSource === b ? b : a)
  const peer = source === a ? b : a
  const [span, setSpan] = useState<LatencyWindow>(DEFAULT_WINDOW)
  const [expanded, setExpanded] = useState(false)
  const { state, refresh, readMore } = useLatencyHistory({ networkId, sourceNodeId: source, peerNodeId: peer, span, enabled: true })
  const history = useMemo(
    () => (state.query ? buildHistory(state.records, state.query, breakAfter(state.expectedIntervalSeconds)) : null),
    [state.query, state.records, state.expectedIntervalSeconds]
  )
  const name = (id: string) => names.get(id) ?? "未知节点"
  const directions = [
    { value: a, label: `${name(a)} → ${name(b)}` },
    { value: b, label: `${name(b)} → ${name(a)}` },
  ]
  const windowLabel = LATENCY_WINDOWS.find((entry) => entry.value === span)?.label ?? ""

  const controls = (
    <div className="flex w-full flex-wrap items-center gap-2 sm:w-auto">
      <div role="group" aria-label="测量方向" className="grid w-full grid-cols-2 gap-0.5 rounded-lg bg-muted p-0.5 sm:w-auto">
        {directions.map((direction) => (
          <button
            key={direction.value}
            type="button"
            aria-pressed={source === direction.value}
            onClick={() => setSource(direction.value)}
            className="min-w-0 truncate rounded-md px-2.5 py-1 text-xs text-muted-foreground outline-none hover:bg-background/60 focus-visible:ring-2 focus-visible:ring-ring/40 aria-pressed:bg-background aria-pressed:font-medium aria-pressed:text-foreground aria-pressed:shadow-xs dark:aria-pressed:bg-input/40"
          >
            {direction.label}
          </button>
        ))}
      </div>
    </div>
  )

  return (
    <section aria-label="延迟历史" data-testid="latency-history" className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-medium">
          延迟历史{" "}
          <span className="font-normal text-muted-foreground" title="认证邻接探测往返耗时，含排队和可能的重试等待；不是纯网络 RTT 或丢包率">
            · 认证邻接往返时间 · 近 {windowLabel}
          </span>
        </h3>
        <Button variant="ghost" size="icon-sm" aria-label="展开延迟历史" onClick={() => setExpanded(true)}>
          <Maximize2Icon />
        </Button>
      </div>
      {controls}
      <HistoryBody state={state} history={history} compact onRetry={refresh} onReadMore={readMore} />

      <Dialog open={expanded} onOpenChange={setExpanded}>
        <DialogContent className="max-h-[90svh] overflow-auto sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>
              {name(source)} <ArrowRightIcon className="inline size-4 align-[-2px]" aria-hidden="true" /> {name(peer)} 延迟历史
            </DialogTitle>
            <DialogDescription>由 {name(source)} 测得的认证邻接探测往返耗时（含排队和可能的重试等待）；均为历史记录，不代表当前连通。</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center gap-2">
              {controls}
              <Segmented
                size="sm"
                aria-label="时间范围"
                value={span}
                onValueChange={setSpan}
                options={LATENCY_WINDOWS.map((entry) => ({ value: entry.value, label: entry.label }))}
                className="sm:w-auto"
              />
              <Button variant="ghost" size="sm" disabled={state.loading} onClick={refresh} className="ml-auto">
                <RefreshCwIcon data-icon="inline-start" className={state.loading ? "animate-spin" : undefined} />
                重新读取
              </Button>
            </div>
            <HistoryBody state={state} history={history} onRetry={refresh} onReadMore={readMore} />
            {history ? <Timeline history={history} /> : null}
          </div>
        </DialogContent>
      </Dialog>
    </section>
  )
}

function HistoryBody({
  state,
  history,
  compact = false,
  onRetry,
  onReadMore,
}: {
  state: LatencyHistoryState
  history: History | null
  compact?: boolean
  onRetry: () => void
  onReadMore: () => void
}) {
  const query = state.query
  const hasData = history !== null && (history.stats.success > 0 || history.events.length > 0)
  return (
    <div className="flex flex-col gap-2">
      {state.error ? (
        <p role="alert" data-testid="latency-error" className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-2.5 py-1.5 text-xs text-destructive">
          <span className="min-w-0 flex-1 break-words">
            {state.error}
            {state.fetchedAt !== null ? `（显示 ${formatClock(state.fetchedAt)} 读取的记录）` : ""}
          </span>
          <Button variant="outline" size="sm" disabled={state.loading} onClick={onRetry}>
            重试
          </Button>
        </p>
      ) : null}
      {query === null ? (
        state.error ? null : (
          <div data-testid="latency-loading" className="flex h-14 items-center justify-center rounded-lg border border-dashed text-xs text-muted-foreground">
            正在读取…
          </div>
        )
      ) : !history || !hasData ? (
        <div data-testid="latency-empty" className="flex min-h-14 flex-col items-center justify-center gap-0.5 rounded-lg border border-dashed px-3 py-2 text-center text-xs text-muted-foreground">
          <span>这段时间没有记录</span>
          <span className="text-[0.6875rem]">未采样或未上报，不代表断开</span>
        </div>
      ) : (
        <>
          <LatencyChart
            history={history}
            from={Date.parse(query.from)}
            until={Date.parse(query.until)}
            height={compact ? 56 : 200}
            interactive={!compact}
            failureLabels={failureReasonLabels}
          />
          <p data-testid="latency-summary" className="text-xs text-muted-foreground tabular-nums">
            成功 {history.stats.success}
            {history.stats.medianMs !== null ? `（中位 ${formatMs(history.stats.medianMs)}，${formatMs(history.stats.minMs ?? 0)}–${formatMs(history.stats.maxMs ?? 0)}）` : ""}
            {` · 超时 ${history.stats.timeout} · 失败 ${history.stats.failed}`}
            {history.segments.length > 1 ? ` · ${history.segments.length} 段` : ""}
          </p>
        </>
      )}
      {query !== null && state.next !== null ? (
        <p role="status" data-testid="latency-truncated" className="flex items-center gap-2 text-xs text-warning">
          <span className="flex-1">只读取了前 {state.records.length} 条（按接收顺序），这段时间还有更多记录。</span>
          <Button variant="outline" size="sm" disabled={state.loading} onClick={onReadMore}>
            继续读取
          </Button>
        </p>
      ) : null}
      {!compact && query !== null ? (
        <p className="text-[0.6875rem] text-muted-foreground">
          {formatDateTime(query.from)} – {formatDateTime(query.until)}
          {state.fetchedAt !== null ? ` · 读取于 ${formatClock(state.fetchedAt)}` : ""}
          {state.expectedIntervalSeconds !== null ? ` · 期望每 ${state.expectedIntervalSeconds} 秒采样一次，邻居多时轮转更慢` : ""}
        </p>
      ) : null}
    </div>
  )
}

/** Segment legend, carrier / configuration changes and pair events, as text. */
function Timeline({ history }: { history: History }) {
  const rows = [
    ...history.changes.map((change) => ({ at: change.at, text: `分段变化：${change.changed.join("、")}`, tone: "text-muted-foreground" })),
    ...history.events.map((event) => ({
      at: event.at,
      text: event.outcome === "timeout" ? "探测超时" : `探测失败${event.reason ? `：${failureReasonLabels[event.reason]}` : ""}`,
      tone: event.outcome === "timeout" ? "text-warning" : "text-destructive",
    })),
  ].sort((x, y) => x.at - y.at)
  return (
    <div className="flex flex-col gap-3 text-xs">
      {history.segments.length > 0 ? (
        <ul aria-label="分段" className="flex flex-wrap gap-x-4 gap-y-1">
          {history.segments.map((segment, index) => (
            <li key={segment.key} className="flex items-center gap-1.5">
              <span aria-hidden="true" className="h-0.5 w-4 rounded-full" style={{ background: SEGMENT_COLORS[index % SEGMENT_COLORS.length] }} />
              {segmentLabel(history, index)}
            </li>
          ))}
        </ul>
      ) : null}
      <section aria-label="事件">
        <h4 className="mb-1 font-medium">事件</h4>
        {rows.length === 0 ? (
          <p className="text-muted-foreground">没有超时、失败或分段变化记录。</p>
        ) : (
          <ol data-testid="latency-events" className="flex flex-col gap-0.5">
            {rows.map((row, index) => (
              <li key={`${row.at}-${index}`} className={row.tone}>
                <time className="mr-2 font-mono tabular-nums">{formatClock(row.at)}</time>
                {row.text}
              </li>
            ))}
          </ol>
        )}
      </section>
      <DiagnosticDetails title={`全部记录（按测量时间，最多 ${LIST_LIMIT} 条）`}>
        <ol data-testid="latency-records" className="flex flex-col gap-0.5 font-mono tabular-nums">
          {[
            ...history.successes.map((point) => ({ at: point.at, id: point.id, text: formatMs(point.ms) })),
            ...history.events.map((event) => ({ at: event.at, id: event.id, text: event.outcome === "timeout" ? "超时" : "失败" })),
          ]
            .sort((x, y) => x.at - y.at)
            .slice(-LIST_LIMIT)
            .map((row) => (
              <li key={row.id}>
                {formatClock(row.at)} {row.text}
              </li>
            ))}
        </ol>
      </DiagnosticDetails>
    </div>
  )
}
