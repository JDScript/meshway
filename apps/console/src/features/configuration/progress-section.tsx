import type {
  ConfigurationAppliedResponse,
  NodePublicationProgress,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useMemo } from "react"

import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { clockTime, dateTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { useAppliedRevision } from "./applied"
import {
  deploymentOf,
  nodePhase,
  publicationSettled,
  type Freshness,
  type NodePhase,
} from "./progress"
import { progressNodesQuery, publicationQuery } from "./progress-queries"
import { isDenied } from "./queries"

const phaseLabel: Record<NodePhase, () => string> = {
  failed: () => m.configuration_progress_failed(),
  applied: () => m.configuration_progress_applied(),
  saved: () => m.configuration_progress_saved(),
  unreported: () => m.configuration_progress_unreported(),
}

const freshnessLabel: Record<Freshness, () => string> = {
  current: () => m.configuration_progress_fresh_current(),
  expired: () => m.configuration_progress_fresh_expired(),
  awaitingInstance: () => m.configuration_progress_fresh_awaiting(),
  superseded: () => m.configuration_progress_fresh_superseded(),
}

const phaseDot: Record<NodePhase, string> = {
  failed: "bg-destructive",
  applied: "bg-foreground",
  saved: "border border-foreground",
  unreported: "border border-muted-foreground",
}

/**
 * A revision's deployment, where this tab applied it: each publication the
 * apply returned, on its own, with its target nodes' receipts as the
 * controller reports them, read again while shown until it settles. For a
 * revision applied elsewhere (or before a reload) the controller provides
 * no way to find its publications; the current revision says so.
 */
export function ApplyProgress({
  networkId,
  revision,
  current,
}: {
  networkId: string
  revision: number
  /** The revision is the network's current one. */
  current: boolean
}) {
  const applied = useAppliedRevision(networkId, revision)
  if (!applied)
    return current ? (
      <Section title={m.configuration_progress()}>
        <EmptyLine>{m.configuration_progress_unavailable()}</EmptyLine>
      </Section>
    ) : null
  return <Applied networkId={networkId} applied={applied} />
}

function Applied({
  networkId,
  applied,
}: {
  networkId: string
  applied: ConfigurationAppliedResponse
}) {
  const deployment = deploymentOf(applied)
  return (
    <Section title={m.configuration_progress()}>
      <div className="grid grid-cols-2 gap-2">
        <Tile label={m.configuration_progress_targets()}>
          <span className="tabular-nums">{applied.targetCount}</span>
        </Tile>
        <Tile label={m.configuration_progress_publications()}>
          <span className="tabular-nums">{applied.publicationIds.length}</span>
        </Tile>
      </div>
      {deployment === "none" ? (
        <EmptyLine>{m.configuration_progress_none()}</EmptyLine>
      ) : deployment === "unpublished" ? (
        <EmptyLine>{m.configuration_progress_unpublished()}</EmptyLine>
      ) : (
        <>
          <p className="text-meta text-muted-foreground">
            {m.configuration_progress_separate()}
          </p>
          <div className="flex flex-col gap-3">
            {applied.publicationIds.map((publicationId) => (
              <Publication
                key={publicationId}
                networkId={networkId}
                publicationId={publicationId}
              />
            ))}
          </div>
        </>
      )}
    </Section>
  )
}

function Publication({
  networkId,
  publicationId,
}: {
  networkId: string
  publicationId: string
}) {
  const progress = useQuery(publicationQuery(networkId, publicationId))
  const nodes = useQuery(progressNodesQuery(networkId))
  const names = useMemo(
    () =>
      new Map(
        (nodes.data?.items ?? []).map((node) => [node.id, node.hostname])
      ),
    [nodes.data]
  )
  const data = progress.data
  const rows = data ? Object.entries(data.nodes) : []
  const settled = data ? publicationSettled(data) : false

  return (
    <div
      className="flex flex-col gap-2 rounded-tile border border-divider p-3"
      data-publication={publicationId}
      data-settled={settled}
    >
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-meta text-muted-foreground">
        <span
          className="font-mono text-foreground"
          title={publicationId}
          translate="no"
        >
          {m.configuration_progress_publication({
            id: publicationId.slice(0, 8),
          })}
        </span>
        {data ? (
          <span className="tabular-nums">
            {m.configuration_progress_sequence({ sequence: data.sequence })}
          </span>
        ) : null}
        {data?.withdrawnAt ? (
          <span className="text-warning">
            {m.configuration_progress_withdrawn({
              time: dateTime(data.withdrawnAt) ?? "—",
            })}
          </span>
        ) : null}
        <span className="flex-1" />
        {data ? (
          <span className="tabular-nums">
            {settled
              ? m.configuration_progress_read_settled({
                  time: clockTime(progress.dataUpdatedAt),
                })
              : m.configuration_progress_read({
                  time: clockTime(progress.dataUpdatedAt),
                })}
          </span>
        ) : null}
      </div>
      {progress.isPending ? (
        <Skeleton className="h-10 w-full rounded-tile" />
      ) : !data ? (
        <EmptyLine size="sm">
          {isDenied(progress.error)
            ? m.configuration_progress_denied()
            : m.configuration_progress_failed_read()}
        </EmptyLine>
      ) : rows.length === 0 ? (
        <EmptyLine size="sm">{m.configuration_progress_no_nodes()}</EmptyLine>
      ) : (
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
      )}
      {data && progress.isError ? (
        <p className="text-meta text-warning">
          {m.configuration_progress_stale()}
        </p>
      ) : null}
    </div>
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
  report: NodePublicationProgress
}) {
  const phase = nodePhase(report)
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
        {report.lastReceivedAt
          ? m.configuration_progress_received({
              time: dateTime(report.lastReceivedAt) ?? "—",
            })
          : m.configuration_progress_never_received()}
      </span>
    </li>
  )
}
