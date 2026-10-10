import type {
  ConfigurationState,
  DerivedNodeState,
  NetmapRef,
  NodeStateView,
  StateView,
} from "@meshway/control-client"
import type { UseQueryResult } from "@tanstack/react-query"
import { useMemo } from "react"

import { failureText } from "@/api/failure"
import { useAddressGeo } from "@/api/queries/ip-geo"
import { CopyButton } from "@/components/chrome/copy-button"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { agoTime, dateTime, relativeTime } from "@/lib/format"
import {
  geoText,
  LOCATION_SOURCE,
  NO_ADDRESSES,
  nodeAddresses,
  type AddressScope,
} from "@/lib/geo"
import {
  configurationFacts,
  desiredAgainst,
  isPast,
  nodePaths,
  reportOf,
  stateOf,
  type DesiredMatch,
} from "@/lib/nodes/state-model"
import {
  applyPhaseLabel,
  locationAbsentLabel,
  pathEvidenceLabel,
  problemText,
  reporterRoleLabel,
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
}: {
  nodeId: string
  /** The server's netmap for the node (the inventory's). */
  desired: NetmapRef | null
  state: UseQueryResult<StateView>
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
          assigned={desired}
          now={now}
        />
      ) : null}
      <ReportedPaths nodeId={nodeId} state={state.data} now={now} />
      <Location derived={view.derived} now={now} />
    </div>
  )
}

/** End-to-end path evidence is distinct from a node's direct neighbors. */
function ReportedPaths({
  nodeId,
  state,
  now,
}: {
  nodeId: string
  state: StateView
  now: number
}) {
  const paths = nodePaths(state, nodeId)
  const name = (id: string) => stateOf(state, id)?.hostname ?? id.slice(0, 8)
  if (!paths.length) return null
  return (
    <Section title={m.nodes_paths()} count={paths.length}>
      <ul className="flex flex-col gap-1">
        {paths.map((path) => (
          <li
            key={`${path.pathId}-${path.reporter.nodeId}`}
            className="flex flex-col gap-0.5 rounded-tile bg-fill-subtle px-3.5 py-2"
          >
            <span className="truncate text-row font-medium">
              {path.state.nodes.map(name).join(" → ")}
            </span>
            <span className="text-meta text-muted-foreground">
              {pathEvidenceLabel[path.state.evidence]()} ·{" "}
              {m.nodes_path_reported_by({
                name: name(path.reporter.nodeId),
                role: reporterRoleLabel[path.state.reporterRole](),
              })}{" "}
              · <When at={path.state.expiresAt} now={now} expiry />
            </span>
          </li>
        ))}
      </ul>
    </Section>
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
  desired: NetmapRef | null
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
          <Tile label={m.nodes_execution_serial()}>
            <span className="font-mono">#{desired.serial}</span>
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
                {desired.digest.slice(0, 12)}
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
  assigned,
  now,
}: {
  configuration: ConfigurationState
  /** The server's netmap for the node: applied is up to date only when it is this one. */
  assigned: NetmapRef | null
  now: number
}) {
  const facts = configurationFacts(configuration, assigned)
  const behind = facts.applied !== null && facts.current === false
  const progress = configuration.progress
  const revision = (value: NetmapRef | null) =>
    value ? (
      <span className="font-mono">
        {m.revision({ revision: value.revision })}
        <span className="font-normal text-muted-foreground">
          {" · "}#{value.serial}
        </span>
      </span>
    ) : (
      <span className="font-normal text-muted-foreground">—</span>
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
                {revision(facts.progress.target)}{" "}
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
  const { location, locationAbsent } = nodeAddresses(derived)
  const address = location?.address ?? null
  const asked = useMemo(
    (): ReadonlyMap<string, AddressScope> =>
      address === null ? NO_ADDRESSES : new Map([[address, "public"]]),
    [address]
  )
  const geoOf = useAddressGeo(asked)
  const geo = address === null ? null : geoOf(address)
  return (
    <Section title={m.nodes_location()}>
      {location ? (
        <div className={GRID}>
          <Tile label={m.nodes_location_address()}>
            <span className="font-mono font-normal">{location.address}</span>
          </Tile>
          <Tile label={m.nodes_location_source()}>
            {LOCATION_SOURCE[location.endpointSource]()}
          </Tile>
          <Tile label={m.nodes_location_observed()}>
            <When at={location.observedAt} now={now} />
          </Tile>
          <Tile label={m.nodes_state_expires()}>
            <When at={location.expiresAt} now={now} expiry />
          </Tile>
          <div className="col-span-2">
            <Tile label={m.nodes_location_place()}>
              {geo ? geoText(geo) : "—"}
            </Tile>
          </div>
        </div>
      ) : (
        <EmptyLine>
          {locationAbsent
            ? locationAbsentLabel[locationAbsent]()
            : m.nodes_location_none()}
        </EmptyLine>
      )}
    </Section>
  )
}
