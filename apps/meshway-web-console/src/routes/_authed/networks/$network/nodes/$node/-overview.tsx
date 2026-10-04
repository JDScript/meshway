import type {
  DerivedNodeState,
  NodeInventoryView,
  StateView,
} from "@meshway/control-client"
import type { UseQueryResult } from "@tanstack/react-query"
import { ArrowUpRightIcon } from "lucide-react"

import { failureText } from "@/api/failure"
import { CopyButton } from "@/components/chrome/copy-button"
import { Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { useNow } from "@/hooks/use-now"
import { agoTime, dateTime } from "@/lib/format"
import { asnText, geoText, placeText } from "@/lib/geo"
import {
  configurationSync,
  nodeLinks,
  reportOf,
  stateOf,
  type ConfigurationSync,
} from "@/lib/nodes/state-model"
import { lastSeen, nodeStatus } from "@/lib/nodes/status"
import { locationAbsentLabel, problemText } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { LiveStatus } from "./-live-status"

const GRID = "grid grid-cols-2 gap-2 xl:grid-cols-4"
const WIDE = "col-span-2"
const muted = (text: string) => (
  <span className="font-normal text-muted-foreground">{text}</span>
)

function syncText(sync: ConfigurationSync) {
  switch (sync.kind) {
    case "current":
      return (
        <>
          {m.nodes_sync_current()}
          {muted(` · ${m.revision({ revision: sync.revision })}`)}
        </>
      )
    case "applied":
      return m.revision({ revision: sync.revision })
    case "applying":
      return m.nodes_sync_applying({ revision: sync.revision })
    case "failed":
      return m.nodes_sync_failed({ revision: sync.revision })
    case "behind":
      return m.nodes_sync_behind({
        applied: sync.applied,
        current: sync.current,
      })
    case "none":
      return m.nodes_sync_none()
    case "unknown":
      return muted("—")
  }
}

const syncTone = (sync: ConfigurationSync) =>
  sync.kind === "failed"
    ? "destructive"
    : sync.kind === "applying" || sync.kind === "behind" || sync.kind === "none"
      ? "warning"
      : undefined

/**
 * What matters about a node first: how it is (its status, whether it runs
 * the latest configuration, its links, its problems), who it is (owner or
 * tags, system, agent, ID) and where it is (its addresses and location).
 * The report's own times and references are on the diagnostics tab.
 */
export function NodeOverview({
  node,
  state,
  currentRevision,
  onTransport,
}: {
  node: NodeInventoryView
  state: UseQueryResult<StateView>
  /** The configuration's current revision, when known. */
  currentRevision: number | undefined
  onTransport: () => void
}) {
  const now = useNow(1_000)
  const read = state.data !== undefined
  const view = stateOf(state.data, node.id)
  const report = reportOf(view)
  const full = report.view === "full" ? report.report : null
  const sync = configurationSync(full?.configuration ?? null, currentRevision)
  const links = nodeLinks(state.data, node.id)
  const connected = links.filter(
    (link) => link.phase === "authenticated"
  ).length
  const problems = full?.problems ?? []

  return (
    <div className="flex flex-col gap-8">
      <Section title={m.nodes_overview_status()}>
        <div className={GRID}>
          <Tile label={m.nodes_status()}>
            {read ? (
              <LiveStatus
                status={nodeStatus(node, view)}
                seen={lastSeen(view)}
              />
            ) : state.isError ? (
              muted(failureText(state.error) ?? m.nodes_state_failed())
            ) : (
              muted("—")
            )}
          </Tile>
          <Tile label={m.nodes_configuration()} tone={syncTone(sync)}>
            {syncText(sync)}
          </Tile>
          <Tile
            label={m.nodes_links()}
            action={
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={m.nodes_open_transport()}
                onClick={onTransport}
              >
                <ArrowUpRightIcon />
              </Button>
            }
          >
            {!read
              ? muted("—")
              : links.length === 0
                ? muted(m.nodes_links_none())
                : m.nodes_links_summary({ connected, total: links.length })}
          </Tile>
          <Tile
            label={m.nodes_problems()}
            tone={problems.length ? "destructive" : undefined}
          >
            {!full
              ? muted("—")
              : problems.length
                ? m.nodes_problems_count({ count: problems.length })
                : m.nodes_problems_none()}
          </Tile>
        </div>
        {problems.length ? (
          <ul className="flex flex-col gap-1">
            {problems.map((problem, index) => (
              <li
                key={index}
                className="flex min-h-12 items-center justify-between gap-3 rounded-tile bg-fill-subtle px-3.5 py-2 text-row"
              >
                <span className="min-w-0 text-destructive">
                  {problemText(problem)}
                </span>
                <span className="shrink-0 text-meta text-muted-foreground">
                  {agoTime(problem.observedAt, now)}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
      </Section>

      <Section title={m.nodes_facts()}>
        <div className={GRID}>
          <div className={WIDE}>
            <Tile
              label={
                node.identity.type === "tag"
                  ? m.nodes_identity_tags()
                  : m.nodes_identity_user()
              }
            >
              {node.identity.type === "tag" ? (
                <span className="font-mono font-normal">
                  {node.identity.tags.join(", ")}
                </span>
              ) : (
                node.identity.user
              )}
            </Tile>
          </div>
          <Tile label={m.nodes_system_os()}>
            {full ? `${full.system.os} · ${full.system.arch}` : muted("—")}
          </Tile>
          <Tile label={m.nodes_system_agent()}>
            {full ? (
              <span className="font-mono font-normal">
                {full.system.agentVersion}
              </span>
            ) : (
              muted("—")
            )}
          </Tile>
          <div className={WIDE}>
            <Tile
              label={m.nodes_id()}
              action={<CopyButton value={node.id} label={m.nodes_id()} />}
            >
              <span className="font-mono font-normal">{node.id}</span>
            </Tile>
          </div>
          <Tile label={m.nodes_system_hostname()}>
            {full ? (
              <span className="font-mono font-normal">
                {full.system.hostname}
              </span>
            ) : (
              muted("—")
            )}
          </Tile>
          <Tile label={m.nodes_system_selection()}>
            {!full
              ? muted("—")
              : (full.selection.appliedProfile ??
                muted(m.nodes_system_selection_none()))}
          </Tile>
          <div className={WIDE}>
            <Tile label={m.nodes_registered()}>
              {dateTime(node.registeredAt) ?? "—"}
              {muted(` · ${agoTime(node.registeredAt, now) ?? ""}`)}
            </Tile>
          </div>
        </div>
      </Section>

      <Section title={m.nodes_addresses()}>
        <div className={GRID}>
          <div className={WIDE}>
            <Address label="IPv4" value={node.addresses.ipv4} />
          </div>
          <div className={WIDE}>
            <Address label="IPv6" value={node.addresses.ipv6} />
          </div>
          <div className={WIDE}>
            <Tile label={m.nodes_location()}>
              {view ? <Place derived={view.derived} /> : muted("—")}
            </Tile>
          </div>
          <div className={WIDE}>
            <Tile label={m.nodes_location_asn()}>
              {view ? <Provider derived={view.derived} /> : muted("—")}
            </Tile>
          </div>
        </div>
        {view && Object.keys(view.derived.ipInfo).length > 1 ? (
          // More than one address: each with where it is and its network.
          <ul className="flex flex-col gap-1">
            {Object.entries(view.derived.ipInfo)
              .sort(([a], [b]) => (a < b ? -1 : 1))
              .map(([ip, info]) => (
                <li
                  key={ip}
                  className="grid grid-cols-[minmax(0,12rem)_minmax(0,1fr)_minmax(0,1fr)] items-baseline gap-x-3 rounded-tile bg-fill-subtle px-3.5 py-2 text-meta"
                >
                  <span className="truncate font-mono">{ip}</span>
                  <span className="truncate">{geoText(info)}</span>
                  <span className="truncate text-muted-foreground">
                    {asnText(info)}
                  </span>
                </li>
              ))}
          </ul>
        ) : null}
      </Section>
    </div>
  )
}

function Place({ derived }: { derived: DerivedNodeState }) {
  if (derived.location)
    return placeText(derived.location.data) ?? m.nodes_geo_empty()
  return muted(
    derived.locationAbsent
      ? locationAbsentLabel[derived.locationAbsent]()
      : m.nodes_location_none()
  )
}

/** The network of the address the location rests on, or of the one address observed. */
function Provider({ derived }: { derived: DerivedNodeState }) {
  const addresses = Object.keys(derived.ipInfo)
  const address =
    derived.location?.address ??
    (addresses.length === 1 ? addresses[0] : undefined)
  const info =
    address !== undefined && Object.hasOwn(derived.ipInfo, address)
      ? derived.ipInfo[address]
      : undefined
  if (!info) return muted("—")
  return info.asn.status === "available" ? asnText(info) : muted(asnText(info))
}

function Address({ label, value }: { label: string; value: string | null }) {
  return (
    <Tile
      label={label}
      action={value ? <CopyButton value={value} label={label} /> : undefined}
    >
      {value ? (
        <span className="font-mono font-normal">{value}</span>
      ) : (
        muted(m.nodes_address_none())
      )}
    </Tile>
  )
}
