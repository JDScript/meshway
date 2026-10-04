import type {
  ConfigurationRef,
  ConfigurationState,
  DerivedNodeState,
  IpInfo,
  NodeStateView,
  StateView,
} from "@meshway/control-client"
import type { UseQueryResult } from "@tanstack/react-query"

import { failureText } from "@/api/failure"
import { CopyButton } from "@/components/chrome/copy-button"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { dateTime, relativeTime } from "@/lib/format"
import {
  configurationFacts,
  desiredAgainst,
  isPast,
  reportOf,
  stateOf,
  type DesiredMatch,
} from "@/lib/nodes/state-model"
import { m } from "@/paraglide/messages.js"

import {
  applyPhaseLabel,
  freshnessLabel,
  locationAbsentLabel,
  problemText,
} from "../-state-labels"

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
 * The node's runtime State as the controller holds it: the report's
 * freshness and times, what the node says about itself (system,
 * configuration, selection, problems) when this account sees its full
 * report, and the location and IP information the controller derives.
 */
export function NodeStateSections({
  nodeId,
  configuredHostname,
  currentRevision,
  desired,
  state,
  now,
}: {
  nodeId: string
  configuredHostname: string
  /** The configuration's current revision, when known. */
  currentRevision: number | undefined
  /** The controller's desired execution target for the node (the inventory's). */
  desired: ConfigurationRef | null
  state: UseQueryResult<StateView>
  now: number
}) {
  if (state.data === undefined)
    return (
      <>
        <Section title={m.nodes_state()}>
          {state.isError ? (
            <EmptyLine>
              {failureText(state.error) ?? m.nodes_state_failed()}
            </EmptyLine>
          ) : (
            <Skeleton className="h-24 w-full rounded-tile" />
          )}
        </Section>
        <Execution desired={desired} reported={null} />
      </>
    )
  const view = stateOf(state.data, nodeId)
  if (!view)
    return (
      <>
        <Section title={m.nodes_state()}>
          <EmptyLine>{m.nodes_state_absent()}</EmptyLine>
        </Section>
        <Execution desired={desired} reported={null} />
      </>
    )
  const report = reportOf(view)
  return (
    <>
      <Section title={m.nodes_state()}>
        <Freshness view={view} now={now} />
      </Section>
      {report.view === "full" ? null : (
        <Execution desired={desired} reported={null} />
      )}
      {report.view === "full" ? (
        <>
          <Section title={m.nodes_system()}>
            <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
              <Tile label={m.nodes_system_hostname()}>
                <span className="font-mono font-normal">
                  {report.report.system.hostname}
                </span>
                {report.report.system.hostname === configuredHostname ? null : (
                  <span className="font-normal text-muted-foreground">
                    {" · "}
                    {m.nodes_system_hostname_configured({
                      name: configuredHostname,
                    })}
                  </span>
                )}
              </Tile>
              <Tile label={m.nodes_system_os()}>
                {report.report.system.os} · {report.report.system.arch}
              </Tile>
              <Tile label={m.nodes_system_agent()}>
                <span className="font-mono font-normal">
                  {report.report.system.agentVersion}
                </span>
              </Tile>
              <Tile label={m.nodes_system_selection()}>
                {report.report.selection.appliedProfile ?? (
                  <span className="font-normal text-muted-foreground">
                    {m.nodes_system_selection_none()}
                  </span>
                )}
              </Tile>
            </div>
          </Section>
          <Execution desired={desired} reported={report.report.configuration} />
          <ConfigurationSection
            facts={configurationFacts(
              report.report.configuration,
              currentRevision
            )}
            progressAt={report.report.configuration.progress?.observedAt}
            progressProblem={report.report.configuration.progress?.problem}
            now={now}
          />
          <Section
            title={m.nodes_problems()}
            count={report.report.problems.length}
          >
            {report.report.problems.length === 0 ? (
              <EmptyLine>{m.nodes_problems_none()}</EmptyLine>
            ) : (
              <ul className="flex flex-col gap-1">
                {report.report.problems.map((problem, index) => (
                  <li
                    key={index}
                    className="flex min-h-12 items-center justify-between gap-3 rounded-tile bg-fill-subtle px-3.5 py-2 text-row"
                  >
                    <span className="min-w-0 text-destructive">
                      {problemText(problem)}
                    </span>
                    <span className="shrink-0 text-meta text-muted-foreground">
                      {relativeTime(problem.observedAt, now)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </>
      ) : report.view === "peer" ? (
        <Section title={m.nodes_system()}>
          <EmptyLine>{m.nodes_state_peer_only()}</EmptyLine>
        </Section>
      ) : null}
      <Location derived={view.derived} now={now} />
    </>
  )
}

function Freshness({ view, now }: { view: NodeStateView; now: number }) {
  return (
    <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
      <Tile
        label={m.nodes_freshness()}
        tone={view.freshness === "current" ? undefined : "warning"}
      >
        {freshnessLabel[view.freshness]()}
      </Tile>
      <Tile label={m.nodes_state_received()}>
        <When at={view.receivedAt} now={now} />
      </Tile>
      <Tile label={m.nodes_state_observed()}>
        <When at={view.observedAt} now={now} />
      </Tile>
      <Tile label={m.nodes_state_expires()}>
        <When at={view.reportExpiresAt} now={now} expiry />
      </Tile>
      <div className="col-span-2">
        <Tile label={m.nodes_state_authenticated()}>
          <When at={view.lastAuthenticatedAt} now={now} />
        </Tile>
      </div>
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
            <span className="font-mono font-normal">
              {view.instanceId}
              {view.reportSequence != null ? ` · #${view.reportSequence}` : ""}
            </span>
          ) : (
            <span className="font-normal text-muted-foreground">—</span>
          )}
        </Tile>
      </div>
    </div>
  )
}

const desiredText: Record<DesiredMatch, () => string> = {
  none: () => m.nodes_execution_none(),
  unreported: () => m.nodes_execution_unreported(),
  applied: () => m.nodes_execution_applied(),
  saved: () => m.nodes_execution_saved(),
  elsewhere: () => m.nodes_execution_elsewhere(),
}

/**
 * What the controller wants the node to run now (its desired execution
 * target), and how that compares with what the node reports: the target
 * is never itself saved or applied.
 */
function Execution({
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
        <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
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
        {desiredText[match]()}
      </p>
    </Section>
  )
}

function ConfigurationSection({
  facts,
  progressAt,
  progressProblem,
  now,
}: {
  facts: ReturnType<typeof configurationFacts>
  progressAt: string | undefined
  progressProblem: Parameters<typeof problemText>[0] | null | undefined
  now: number
}) {
  const revision = (value: number | null) =>
    value === null ? (
      <span className="font-normal text-muted-foreground">—</span>
    ) : (
      <span className="font-mono">{m.revision({ revision: value })}</span>
    )
  return (
    <Section title={m.nodes_configuration()}>
      <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
        <Tile label={m.nodes_configuration_saved()}>
          {revision(facts.saved)}
        </Tile>
        <Tile
          label={m.nodes_configuration_applied()}
          tone={facts.current === false ? "warning" : undefined}
        >
          {revision(facts.applied)}
          {facts.current === false ? (
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
                {progressAt ? (
                  <span className="font-normal text-muted-foreground">
                    {" · "}
                    {relativeTime(progressAt, now)}
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
      {progressProblem ? (
        <p className="text-row text-destructive">
          {problemText(progressProblem)}
        </p>
      ) : null}
    </Section>
  )
}

function ipInfoText(ip: string, info: IpInfo): string {
  const geo =
    info.geoip.status === "available"
      ? [info.geoip.data.country, info.geoip.data.region, info.geoip.data.city]
          .filter(Boolean)
          .join(" · ") || m.nodes_geo_empty()
      : info.geoip.status === "notFound"
        ? m.nodes_geo_not_found()
        : info.geoip.status === "unavailable"
          ? locationAbsentLabel[info.geoip.reason]()
          : m.nodes_geo_not_applicable()
  const asn =
    info.asn.status === "available"
      ? `AS${info.asn.data.number}${info.asn.data.organization ? ` ${info.asn.data.organization}` : ""}`
      : info.asn.status === "notFound"
        ? m.nodes_asn_not_found()
        : info.asn.status === "unavailable"
          ? m.nodes_asn_unavailable()
          : m.nodes_asn_not_applicable()
  return `${ip} · ${geo} · ${asn}`
}

function Location({
  derived,
  now,
}: {
  derived: DerivedNodeState
  now: number
}) {
  const location = derived.location
  const ips = Object.entries(derived.ipInfo).sort(([a], [b]) =>
    a < b ? -1 : 1
  )
  return (
    <Section title={m.nodes_location()}>
      {location ? (
        <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
          <div className="col-span-2">
            <Tile label={m.nodes_location_region()}>
              {[location.data.country, location.data.region, location.data.city]
                .filter(Boolean)
                .join(" · ") || m.nodes_geo_empty()}
            </Tile>
          </div>
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
          <Tile label={m.nodes_location_expires()}>
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
      {ips.length > 0 ? (
        <ul className="flex flex-col gap-1">
          {ips.map(([ip, info]) => (
            <li
              key={ip}
              className="truncate rounded-tile bg-fill-subtle px-3.5 py-2 font-mono text-meta"
            >
              {ipInfoText(ip, info)}
            </li>
          ))}
        </ul>
      ) : null}
    </Section>
  )
}
