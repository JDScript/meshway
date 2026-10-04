import {
  cancelObservation,
  type ObservationTask,
} from "@meshway/control-client"
import { useMutation } from "@tanstack/react-query"

import { authed, client } from "@/api/client"
import { failureText, toastFailure, type FailureText } from "@/api/failure"
import { BusyLabel } from "@/components/chrome/busy-label"
import { CopyButton } from "@/components/chrome/copy-button"
import { EmptyLine } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { relativeTime } from "@/lib/format"
import { millis, percent } from "@/lib/history/format"
import type { SegmentSample, Standing } from "@/lib/observation/record"
import {
  gapFor,
  seriesPoints,
  totals,
  unansweredRate,
} from "@/lib/observation/series"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { metricLabel, phaseLabel, serviceText } from "./labels"
import { ObservationChart } from "./observation-chart"
import { useSubscription } from "./use-subscription"

/** What a refused renewal means. */
const renewFailureText: FailureText = {
  observation_limit: () => m.observation_renew_limit(),
  not_found: () => m.observation_removed(),
  permission_denied: () => m.observation_renew_denied(),
  state_conflict: () => m.observation_renew_conflict(),
}

/** The chart's stretch: a minute, or sixty periods, at most half an hour. */
const spanFor = (intervalMs: number) =>
  Math.min(30 * 60_000, Math.max(60_000, intervalMs * 60))

function standingText(standing: Standing): string {
  switch (standing.type) {
    case "unknown":
      return ""
    case "active":
      return phaseLabel[standing.phase]()
    case "lapsed":
      return m.observation_lapsed()
    case "ended":
      return standing.phase
        ? phaseLabel[standing.phase]()
        : m.observation_removed()
  }
}

/**
 * The observation this tab holds: what the controller says of it (phase,
 * path, lease) and the samples it delivered since this tab followed it.
 */
export function LiveObservation({
  networkId,
  subscriptionId,
  nameOf,
  onClose,
}: {
  networkId: string
  subscriptionId: string
  nameOf: (id: string) => string
  onClose: () => void
}) {
  const { query, record, standing, visible, renewFailure, now } =
    useSubscription(networkId, subscriptionId)
  const stop = useMutation({
    mutationFn: () =>
      authed((token) =>
        cancelObservation(client, { networkId, subscriptionId }, { token })
      ),
    onSuccess: () => void query.refetch(),
    onError: toastFailure,
  })
  const task = record?.task ?? null
  const active = standing.type === "active"

  if (!task)
    return query.isError ? (
      <EmptyLine>
        {failureText(query.error, renewFailureText) ?? m.observation_failed()}
      </EmptyLine>
    ) : (
      <Skeleton className="h-36 w-full rounded-tile" />
    )

  const samples = record?.samples ?? []
  const ended = standing.type === "ended" || standing.type === "lapsed"

  return (
    <section className="flex flex-col gap-3">
      <div className="flex min-h-8 flex-wrap items-center gap-x-3 gap-y-1">
        <h3 className="min-w-0 truncate text-row font-medium">
          → {nameOf(task.targetNodeId)}
        </h3>
        <span
          className={cn(
            "text-meta",
            standing.type === "active" && standing.phase === "running"
              ? "text-foreground"
              : ended &&
                  (standing.type === "lapsed" ||
                    (standing.type === "ended" &&
                      (standing.phase === "failed" ||
                        standing.phase === "unsupported")))
                ? "text-warning"
                : "text-muted-foreground"
          )}
        >
          {standingText(standing)}
        </span>
        <span className="flex-1" />
        {active ? (
          <Button
            variant="outline"
            size="sm"
            disabled={stop.isPending}
            onClick={() => stop.mutate()}
          >
            <BusyLabel busy={stop.isPending}>{m.observation_stop()}</BusyLabel>
          </Button>
        ) : null}
        <Button variant="ghost" size="sm" onClick={onClose}>
          {m.observation_close()}
        </Button>
      </div>

      <div className="text-meta text-muted-foreground">
        {[
          serviceText(task.service),
          task.selection.mode === "auto"
            ? m.observation_selection_auto()
            : m.observation_selection_explicit(),
          m.observation_every({ value: String(task.intervalMs / 1000) }),
          m.observation_revision({
            revision: String(task.configuration.revision),
          }),
        ].join(" · ")}
      </div>

      <Facts
        task={task}
        standing={standing}
        visible={visible}
        now={now}
        nameOf={nameOf}
      />

      {task.phase === "unsupported" ? (
        <Note tone="warning">{m.observation_unsupported_detail()}</Note>
      ) : null}
      {task.unsupportedMetrics.length ? (
        <Note>
          {m.observation_metrics_unsupported({
            metrics: task.unsupportedMetrics
              .map((metric) => metricLabel[metric]())
              .join("、"),
          })}
        </Note>
      ) : null}
      {renewFailure ? (
        <Note tone="warning">
          {failureText(renewFailure, renewFailureText)}
        </Note>
      ) : null}

      {ended && samples.length === 0 ? (
        <EmptyLine>{m.observation_no_samples()}</EmptyLine>
      ) : (
        <Measurements
          samples={samples}
          intervalMs={task.intervalMs}
          now={now}
          label={m.observation_chart({ target: nameOf(task.targetNodeId) })}
        />
      )}
    </section>
  )
}

function Measurements({
  samples,
  intervalMs,
  now,
  label,
}: {
  samples: readonly SegmentSample[]
  intervalMs: number
  now: number
  label: string
}) {
  const points = seriesPoints(samples)
  const t = totals(samples)
  const rate = unansweredRate(t)
  const toMs = Math.max(now, points.at(-1)?.atMs ?? 0)
  const fromMs = toMs - spanFor(intervalMs)
  return (
    <>
      <div className="grid grid-cols-2 gap-2 xl:grid-cols-5">
        <Tile label={m.observation_latest()}>
          {t.lastUs !== null ? millis(t.lastUs) : "—"}
        </Tile>
        <Tile label={m.observation_mean()}>
          {t.meanUs !== null ? millis(t.meanUs) : "—"}
        </Tile>
        <Tile label={m.observation_range()}>
          {t.minUs !== null && t.maxUs !== null
            ? `${millis(t.minUs)} – ${millis(t.maxUs)}`
            : "—"}
        </Tile>
        <Tile label={m.observation_samples()}>{t.samples}</Tile>
        <Tile
          label={m.observation_unanswered()}
          tone={rate !== null && rate > 0 ? "warning" : undefined}
        >
          {rate !== null ? percent(rate) : "—"}
        </Tile>
      </div>
      <div className="text-meta text-muted-foreground tabular-nums">
        {m.observation_counts({
          replied: String(t.replied),
          timeout: String(t.timeout),
          failed: String(t.failed),
          pending: String(t.pending),
        })}
      </div>
      <div className="rounded-tile bg-fill-subtle px-3.5 pt-3 pb-2">
        <ObservationChart
          points={points}
          fromMs={fromMs}
          toMs={toMs}
          gapMs={gapFor(intervalMs)}
          label={label}
        />
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-meta text-muted-foreground">
        <Legend swatch="line">{m.observation_legend_reply()}</Legend>
        <Legend swatch="warning">{m.observation_legend_timeout()}</Legend>
        <Legend swatch="muted">{m.observation_legend_failed()}</Legend>
        <span>{m.observation_legend_gap()}</span>
      </div>
    </>
  )
}

function Facts({
  task,
  standing,
  visible,
  now,
  nameOf,
}: {
  task: ObservationTask
  standing: Standing
  visible: boolean
  now: number
  nameOf: (id: string) => string
}) {
  const active = standing.type === "active"
  const expires = task.subscriberExpiresAt ?? task.expiresAt
  return (
    <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
      <div className="col-span-2 min-w-0 xl:col-span-1">
        <Tile label={m.observation_selected_path()}>
          {task.path ? (
            <span title={task.path.map(nameOf).join(" → ")}>
              {task.path.map(nameOf).join(" → ")}
              {task.pathSegment != null ? (
                <span className="text-muted-foreground">
                  {" · "}
                  {m.observation_segment({ value: String(task.pathSegment) })}
                </span>
              ) : null}
            </span>
          ) : (
            <span className="text-muted-foreground">
              {m.observation_path_pending()}
            </span>
          )}
        </Tile>
      </div>
      <Tile label={m.observation_lease()}>
        {active ? relativeTime(expires, now) : m.observation_lease_ended()}
      </Tile>
      <Tile label={m.observation_renew()}>
        {!active
          ? "—"
          : visible
            ? (relativeTime(task.renewAfter, now) ?? "—")
            : m.observation_hidden()}
      </Tile>
      <Tile
        label={m.observation_subscription()}
        action={
          <CopyButton
            value={task.subscriptionId ?? task.id}
            label={m.observation_subscription()}
          />
        }
      >
        <span className="font-mono text-meta">
          {(task.subscriptionId ?? task.id).slice(0, 8)}
        </span>
      </Tile>
    </div>
  )
}

function Note({
  tone,
  children,
}: {
  tone?: "warning"
  children: string | null
}) {
  if (!children) return null
  return (
    <div
      className={cn(
        "text-meta",
        tone === "warning" ? "text-warning" : "text-muted-foreground"
      )}
    >
      {children}
    </div>
  )
}

function Legend({
  swatch,
  children,
}: {
  swatch: "line" | "warning" | "muted"
  children: string
}) {
  return (
    <span className="flex items-center gap-1.5">
      <span
        aria-hidden="true"
        className={cn(
          swatch === "line"
            ? "h-0.5 w-3 rounded-full bg-[var(--signal)]"
            : "h-2 w-[3px] rounded-[1px]",
          swatch === "warning" && "bg-warning",
          swatch === "muted" && "bg-muted-foreground/70"
        )}
      />
      {children}
    </span>
  )
}
