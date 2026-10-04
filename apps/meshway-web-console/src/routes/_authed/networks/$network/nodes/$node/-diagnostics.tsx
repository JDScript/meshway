import type {
  ConfigurationRef,
  ConfigurationState,
  DerivedNodeState,
  NodeStateView,
  StateView,
} from "@meshway/control-client"
import type { UseQueryResult } from "@tanstack/react-query"

import { failureText } from "@/api/failure"
import { CopyButton } from "@/components/chrome/copy-button"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { agoTime, dateTime, relativeTime } from "@/lib/format"
import {
  configurationFacts,
  desiredAgainst,
  isPast,
  reportOf,
  stateOf,
  type DesiredMatch,
} from "@/lib/nodes/state-model"
import {
  applyPhaseLabel,
  locationAbsentLabel,
  problemText,
} from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

const GRID = "grid grid-cols-2 gap-2 xl:grid-cols-4"

function When({
  at,
  now,
  expiry = false,
}: {
  at: string | null | undefined
  now: number
  expiry?: boolean
}) {
  if (!at) return <span className="font-normal text-muted-foreground">—</span>
  const past = expiry && isPast(at, now)
  return (
    <span className={past ? "text-warning" : undefined} title={at}>
      {dateTime(at) ?? at}
      <span className="font-normal text-muted-foreground">
        {" · "}
        {past ? m.nodes_time_past() : relativeTime(at, now)}
      </span>
    </span>
  )
}

/**
 * What the status rests on: the report's times and instance, the
 * configuration the server assigns against what the node saved and
 * applied, and what the location was derived from.
 */
export function NodeDiagnostics({
  nodeId,
  desired,
  state,
  currentRevision,
}: {
  nodeId: string
  /** The server's assigned configuration for the node (the inventory's). */
  desired: ConfigurationRef | null
  state: UseQueryResult<StateView>
  currentRevision: number | undefined
}) {
  const now = useNow(1_000)
  if (state.data === undefined)
    return (
      <Section title={m.nodes_report()}>
        {state.isError ? (
          <EmptyLine>
            {failureText(state.error) ?? m.nodes_state_failed()}
          </EmptyLine>
        ) : (
          <Skeleton className="h-24 w-full rounded-tile" />
        )}
      </Section>
    )
  const view = stateOf(state.data, nodeId)
  if (!view)
    return (
      <div className="flex flex-col gap-8">
        <Section title={m.nodes_report()}>
          <EmptyLine>{m.nodes_state_absent()}</EmptyLine>
        </Section>
        <Assigned desired={desired} reported={null} />
      </div>
    )
  const report = reportOf(view)
  const configuration =
    report.view === "full" ? report.report.configuration : null
  return (
    <div className="flex flex-col gap-8">
      <Section title={m.nodes_report()}>
        <Report view={view} now={now} />
        {report.view === "peer" ? (
          <EmptyLine>{m.nodes_state_peer_only()}</EmptyLine>
        ) : null}
      </Section>
      <Assigned desired={desired} reported={configuration} />
      {configuration ? (
        <Configuration
          configuration={configuration}
          currentRevision={currentRevision}
          now={now}
        />
      ) : null}
      <Location derived={view.derived} now={now} />
    </div>
  )
}

function Report({ view, now }: { view: NodeStateView; now: number }) {
  return (
    <div className={GRID}>
      <Tile label={m.nodes_state_received()}>
        <When at={view.receivedAt} now={now} />
      </Tile>
      <Tile label={m.nodes_state_observed()}>
        <When at={view.observedAt} now={now} />
      </Tile>
      <Tile label={m.nodes_state_expires()}>
        <When at={view.reportExpiresAt} now={now} expiry />
      </Tile>
      <Tile label={m.nodes_state_authenticated()}>
        <When at={view.lastAuthenticatedAt} now={now} />
      </Tile>
      <div className="col-span-2">
        <Tile
          label={m.nodes_state_instance()}
          action={
            view.instanceId ? (
              <CopyButton
                value={view.instanceId}
                label={m.nodes_state_instance()}
              />
            ) : undefined
          }
        >
          {view.instanceId ? (
            <span className="font-mono font-normal">{view.instanceId}</span>
          ) : (
            <span className="font-normal text-muted-foreground">—</span>
          )}
        </Tile>
      </div>
      <Tile label={m.nodes_state_sequence()}>
        {view.reportSequence ?? (
          <span className="font-normal text-muted-foreground">—</span>
        )}
      </Tile>
    </div>
  )
}

const assignedText: Record<DesiredMatch, () => string> = {
  none: () => m.nodes_execution_none(),
  unreported: () => m.nodes_execution_unreported(),
  applied: () => m.nodes_execution_applied(),
  saved: () => m.nodes_execution_saved(),
  elsewhere: () => m.nodes_execution_elsewhere(),
}

/** The configuration the server assigns the node, and whether the node took it. */
function Assigned({
  desired,
  reported,
}: {
  desired: ConfigurationRef | null
  /** The node's own report of its configuration; null without a full report. */
  reported: ConfigurationState | null
}) {
  const match = desiredAgainst(desired, reported)
  return (
    <Section title={m.nodes_execution()}>
      {desired ? (
        <div className={GRID}>
          <Tile label={m.nodes_execution_revision()}>
            <span className="font-mono">
              {m.revision({ revision: desired.revision })}
            </span>
          </Tile>
          <Tile
            label={m.nodes_execution_publication()}
            action={
              <CopyButton
                value={desired.executionPublication}
                label={m.nodes_execution_publication()}
              />
            }
          >
            <span className="font-mono font-normal">
              {desired.executionPublication.slice(0, 8)}
            </span>
          </Tile>
          <div className="col-span-2">
            <Tile
              label={m.nodes_execution_digest()}
              action={
                <CopyButton
                  value={desired.digest}
                  label={m.nodes_execution_digest()}
                />
              }
            >
              <span className="font-mono font-normal">
                {desired.digest.slice(0, 16)}…
              </span>
            </Tile>
          </div>
        </div>
      ) : null}
      <p
        className={
          match === "elsewhere"
            ? "text-row text-warning"
            : "text-row text-muted-foreground"
        }
      >
        {assignedText[match]()}
      </p>
    </Section>
  )
}

function Configuration({
  configuration,
  currentRevision,
  now,
}: {
  configuration: ConfigurationState
  currentRevision: number | undefined
  now: number
}) {
  const facts = configurationFacts(configuration, currentRevision)
  const behind = facts.applied !== null && facts.current === false
  const progress = configuration.progress
  const revision = (value: number | null) =>
    value === null ? (
      <span className="font-normal text-muted-foreground">—</span>
    ) : (
      <span className="font-mono">{m.revision({ revision: value })}</span>
    )
  return (
    <Section title={m.nodes_configuration()}>
      <div className={GRID}>
        <Tile label={m.nodes_configuration_saved()}>
          {revision(facts.saved)}
        </Tile>
        <Tile
          label={m.nodes_configuration_applied()}
          tone={behind ? "warning" : undefined}
        >
          {revision(facts.applied)}
          {behind ? (
            <span className="font-normal">
              {" · "}
              {m.nodes_configuration_behind()}
            </span>
          ) : null}
        </Tile>
        <div className="col-span-2">
          <Tile label={m.nodes_configuration_progress()}>
            {facts.progress ? (
              <>
                <span className="font-mono">
                  {m.revision({ revision: facts.progress.revision })}
                </span>{" "}
                {applyPhaseLabel[facts.progress.phase]()}
                {progress ? (
                  <span className="font-normal text-muted-foreground">
                    {" · "}
                    {agoTime(progress.observedAt, now)}
                  </span>
                ) : null}
              </>
            ) : (
              <span className="font-normal text-muted-foreground">
                {m.nodes_configuration_progress_none()}
              </span>
            )}
          </Tile>
        </div>
      </div>
      {progress?.problem ? (
        <p className="text-row text-destructive">
          {problemText(progress.problem)}
        </p>
      ) : null}
    </Section>
  )
}

function Location({
  derived,
  now,
}: {
  derived: DerivedNodeState
  now: number
}) {
  const location = derived.location
  return (
    <Section title={m.nodes_location()}>
      {location ? (
        <div className={GRID}>
          <Tile label={m.nodes_location_address()}>
            <span className="font-mono font-normal">{location.address}</span>
          </Tile>
          <Tile label={m.nodes_location_source()}>
            {location.endpointSource === "stun"
              ? m.nodes_location_source_stun()
              : m.nodes_location_source_interface()}
          </Tile>
          <Tile label={m.nodes_location_observed()}>
            <When at={location.observedAt} now={now} />
          </Tile>
          <Tile label={m.nodes_state_expires()}>
            <When at={location.expiresAt} now={now} expiry />
          </Tile>
          <div className="col-span-2">
            <Tile label={m.nodes_location_dataset()}>
              <span className="font-mono font-normal">
                {location.datasetId}
              </span>
            </Tile>
          </div>
        </div>
      ) : (
        <EmptyLine>
          {derived.locationAbsent
            ? locationAbsentLabel[derived.locationAbsent]()
            : m.nodes_location_none()}
        </EmptyLine>
      )}
    </Section>
  )
}
