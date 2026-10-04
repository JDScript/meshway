import type {
  ApplicationPhase,
  ConfigurationNodeProgress,
  ReceiptFreshness,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useMemo } from "react"

import { isDenied } from "@/api/queries/configuration"
import {
  configurationProgressQuery,
  PROGRESS_MS,
} from "@/api/queries/configuration-progress"
import { nodesQuery } from "@/api/queries/nodes"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { clockTime, dateTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

const phaseLabel: Record<ApplicationPhase, () => string> = {
  applied: () => m.configuration_progress_applied(),
  saved: () => m.configuration_progress_saved(),
  pending: () => m.configuration_progress_pending(),
  failed: () => m.configuration_progress_failed(),
}

const PHASES: readonly ApplicationPhase[] = [
  "applied",
  "saved",
  "pending",
  "failed",
]

const freshnessLabel: Record<ReceiptFreshness, () => string> = {
  current: () => m.configuration_progress_fresh_current(),
  expired: () => m.configuration_progress_fresh_expired(),
  awaitingInstance: () => m.configuration_progress_fresh_awaiting(),
  superseded: () => m.configuration_progress_fresh_superseded(),
}

const phaseDot: Record<ApplicationPhase, string> = {
  failed: "bg-destructive",
  applied: "bg-foreground",
  saved: "border border-foreground",
  pending: "border border-muted-foreground",
}

/**
 * One revision's deployment as the controller counts it: its fixed target
 * nodes, each counted once, and each node's last accepted phase with how
 * current that report is. The counts are the controller's, never summed
 * here. The event stream keeps the current revision's up to date; another
 * revision's is read again while shown, until the controller refuses it.
 */
export function ApplyProgress({
  networkId,
  revision,
  current,
}: {
  networkId: string
  revision: number
  /** The network's current revision, the one the event stream carries. */
  current: boolean
}) {
  const query = configurationProgressQuery(networkId, revision)
  const progress = useQuery(
    current
      ? query
      : {
          ...query,
          refetchInterval: ({ state }) =>
            isDenied(state.error) ? false : PROGRESS_MS,
        }
  )
  const nodes = useQuery(nodesQuery(networkId))
  const names = useMemo(
    () =>
      new Map(
        (nodes.data?.items ?? []).map((node) => [node.id, node.hostname])
      ),
    [nodes.data]
  )
  const data = progress.data
  const rows = data ? Object.entries(data.nodes) : []

  return (
    <Section
      title={m.configuration_progress()}
      count={
        data
          ? m.configuration_progress_targets({
              count: data.summary.targetCount,
            })
          : null
      }
      action={
        data ? (
          <span className="text-meta text-muted-foreground tabular-nums">
            {m.configuration_progress_updated({
              time: clockTime(progress.dataUpdatedAt),
            })}
          </span>
        ) : null
      }
    >
      {progress.isPending ? (
        <Skeleton className="h-16 w-full rounded-tile" />
      ) : !data ? (
        <EmptyLine>
          {isDenied(progress.error)
            ? m.configuration_progress_denied()
            : m.configuration_progress_failed_read()}
        </EmptyLine>
      ) : data.summary.targetCount === 0 ? (
        <EmptyLine>{m.configuration_progress_none()}</EmptyLine>
      ) : (
        <div
          className="flex flex-col gap-3"
          data-progress-revision={data.revision}
          data-scope={data.scope}
        >
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
            {PHASES.map((phase) => (
              <Tile key={phase} label={phaseLabel[phase]()}>
                <span className="tabular-nums" data-count={phase}>
                  {data.summary[phase]}
                </span>
              </Tile>
            ))}
          </div>
          <p className="text-meta text-muted-foreground">
            {data.scope === "partial"
              ? `${m.configuration_progress_partial()} `
              : ""}
            {m.configuration_progress_not_health()}
          </p>
          <ul className="flex flex-col gap-1">
            {rows.map(([nodeId, report]) => (
              <NodeRow
                key={nodeId}
                nodeId={nodeId}
                name={names.get(nodeId) ?? null}
                report={report}
              />
            ))}
          </ul>
        </div>
      )}
      {data && progress.isError ? (
        <p className="text-meta text-warning">
          {m.configuration_progress_stale()}
        </p>
      ) : null}
    </Section>
  )
}

function NodeRow({
  nodeId,
  name,
  report,
}: {
  nodeId: string
  /** null when the node is not (or no longer) listed. */
  name: string | null
  report: ConfigurationNodeProgress
}) {
  const { phase } = report
  return (
    <li
      className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-tile bg-fill-subtle px-3 py-2 text-row"
      data-node={nodeId}
      data-phase={phase}
      data-freshness={report.freshness}
    >
      <span
        aria-hidden="true"
        className={cn("size-2 shrink-0 rounded-full", phaseDot[phase])}
      />
      <span
        className={cn(
          "min-w-0 flex-1 truncate",
          name === null && "font-mono text-meta"
        )}
        title={name === null ? nodeId : `${name} · ${nodeId}`}
      >
        {name ?? nodeId}
      </span>
      <span className="font-medium">{phaseLabel[phase]()}</span>
      <span className="text-meta text-muted-foreground">
        {freshnessLabel[report.freshness]()}
      </span>
      <span className="w-full text-meta text-muted-foreground tabular-nums sm:w-auto">
        {report.receivedAt
          ? m.configuration_progress_received({
              time: dateTime(report.receivedAt) ?? "—",
            })
          : m.configuration_progress_never_received()}
      </span>
    </li>
  )
}
