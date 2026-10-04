import type { HistoryCoverage } from "@meshway/control-client"
import { RefreshCwIcon } from "lucide-react"

import { IconAction } from "@/components/chrome/icon-action"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { dateTime } from "@/lib/format"
import { rangeText } from "@/lib/history/format"
import { coverageFacts } from "@/lib/history/series"
import {
  WINDOWS,
  type HistoryWindow,
  type WindowChoice,
} from "@/lib/history/window"
import { m } from "@/paraglide/messages.js"

const windowLabel: Record<WindowChoice, () => string> = {
  "1h": () => m.history_window_1h(),
  "6h": () => m.history_window_6h(),
  "24h": () => m.history_window_24h(),
  "7d": () => m.history_window_7d(),
}

const OFF = "off"

/** The window choice; with `offLabel`, an "off" choice before the windows. */
export function WindowChoices({
  value,
  offLabel,
  onChange,
}: {
  value: WindowChoice | null
  offLabel?: string
  onChange: (choice: WindowChoice | null) => void
}) {
  const options = [
    ...(offLabel ? [{ value: OFF, label: offLabel }] : []),
    ...WINDOWS.map((choice) => ({
      value: choice,
      label: windowLabel[choice](),
    })),
  ]
  return (
    <SegmentedControl
      size="sm"
      label={m.history_window()}
      value={value ?? OFF}
      options={options}
      onChange={(next) =>
        onChange(WINDOWS.find((choice) => choice === next) ?? null)
      }
    />
  )
}

/** The window's span as read, and reading the latest one instead. */
export function WindowRange({
  window,
  onRefresh,
  busy,
}: {
  window: HistoryWindow
  onRefresh: () => void
  busy: boolean
}) {
  return (
    <div className="flex h-7 min-w-0 items-center gap-1 text-meta text-muted-foreground tabular-nums">
      <span className="min-w-0 flex-1 truncate">
        {rangeText(window.fromMs, window.toMs)}
      </span>
      <IconAction
        label={m.history_refresh()}
        icon={RefreshCwIcon}
        onClick={onRefresh}
        disabled={busy}
        className="size-7"
      />
    </div>
  )
}

/** What the window does not cover, as the controller states it. */
export function CoverageNotes({
  coverage,
  window,
  nameOf,
}: {
  coverage: readonly HistoryCoverage[]
  window: HistoryWindow
  nameOf: (nodeId: string) => string
}) {
  const facts = coverageFacts(coverage, window)
  if (!facts.length) return null
  return (
    <div className="flex flex-col gap-1">
      <div className="text-meta font-medium text-muted-foreground">
        {m.history_coverage()}
      </div>
      <ul className="flex flex-col gap-1 text-meta text-warning">
        {facts.map((fact) => (
          <li
            key={
              fact.type === "pruned"
                ? `p${fact.sourceNodeId}${fact.reason}`
                : `g${fact.sourceNodeId}${fact.fromMs}${fact.toMs}`
            }
          >
            {fact.type === "pruned"
              ? (fact.reason === "retention"
                  ? m.history_coverage_retention
                  : m.history_coverage_capacity)({
                  node: nameOf(fact.sourceNodeId),
                  time: dateTime(fact.beforeMs) ?? "—",
                })
              : m.history_coverage_gap({
                  node: nameOf(fact.sourceNodeId),
                  range: rangeText(fact.fromMs, fact.toMs),
                  count: String(fact.dropped),
                })}
          </li>
        ))}
      </ul>
    </div>
  )
}
