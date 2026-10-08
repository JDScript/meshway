import type {
  NetworkSummary,
  NodeInventoryView,
  StateView,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import type { UseQueryResult } from "@tanstack/react-query"
import { ArrowUpRightIcon } from "lucide-react"

import { failureText } from "@/api/failure"
import { useAddressGeo } from "@/api/queries/ip-geo"
import { CopyButton } from "@/components/chrome/copy-button"
import { FactList, FactRow } from "@/components/chrome/fact-list"
import { EmptyLine, Section } from "@/components/chrome/section"
import { useNow } from "@/hooks/use-now"
import { agoTime } from "@/lib/format"
import { asnText, geoText, NO_ADDRESSES, nodeAddresses } from "@/lib/geo"
import { countBy } from "@/lib/network-map"
import {
  CONNECTION_STATUS_ORDER,
  nodeConnections,
} from "@/lib/nodes/connections"
import {
  configurationSync,
  desiredAgainst,
  fullReport,
  stateOf,
  type ConfigurationSync,
} from "@/lib/nodes/state-model"
import {
  connectionStatusLabel,
  locationAbsentLabel,
  problemText,
} from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { useNodeConfiguration } from "./-draft"

function syncText(sync: ConfigurationSync, currentTarget: boolean) {
  switch (sync.kind) {
    case "current":
      return currentTarget ? m.nodes_sync_current() : m.nodes_sync_pending()
    case "applied":
      return m.nodes_sync_applied({ revision: sync.revision })
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
      return "—"
  }
}

/** Everyday access and connectivity; registration and runtime evidence have their own places. */
export function NodeOverview({
  node,
  state,
  currentRevision,
  onTransport,
  networkId,
  network,
}: {
  node: NodeInventoryView
  state: UseQueryResult<StateView>
  currentRevision: number | undefined
  onTransport: () => void
  networkId: string
  network: NetworkSummary | null
}) {
  const now = useNow(1_000)
  const view = stateOf(state.data, node.id)
  const report = fullReport(view)
  const sync = configurationSync(report?.configuration ?? null, currentRevision)
  const targetApplied =
    desiredAgainst(node.desiredExecution, report?.configuration ?? null) ===
    "applied"
  const current = targetApplied && view?.freshness === "current"
  const connections = nodeConnections(state.data?.links ?? [], node.id)
  const connected = connections.filter(
    (connection) => connection.status === "connected"
  ).length
  const otherStates = countBy(
    connections
      .filter((connection) => connection.status !== "connected")
      .map((connection) => connection.status),
    CONNECTION_STATUS_ORDER
  )
  const addresses = view ? nodeAddresses(view.derived) : null
  const geoOf = useAddressGeo(addresses?.scopes ?? NO_ADDRESSES)
  const ips = [...(addresses?.scopes ?? NO_ADDRESSES)]
    .filter(([, scope]) => scope === "public")
    .map(([ip]) => ip)
    .sort()
  const configuration = useNodeConfiguration(networkId, node.id, network).state
  const config =
    configuration.kind === "manager" ? configuration.view.configuration : null
  const configuredName = config?.nodes[node.id]?.hostname
  const dnsName =
    configuredName && config ? `${configuredName}.${config.dns.domain}` : null
  const dnsEnabled = config ? config.dns.magicDNS : null
  const attention =
    sync.kind === "failed" ||
    sync.kind === "behind" ||
    sync.kind === "applying" ||
    sync.kind === "none" ||
    (sync.kind === "current" &&
      node.desiredExecution !== null &&
      !targetApplied)

  return (
    <div className="flex flex-col gap-8">
      {state.isError ? (
        <EmptyLine>
          {failureText(state.error) ?? m.nodes_state_failed()}
        </EmptyLine>
      ) : null}
      <>
        <Section title={m.nodes_mesh_access()}>
          <FactList>
            <Address
              label={m.nodes_dns_name()}
              value={dnsName}
              note={dnsEnabled === false ? m.nodes_dns_disabled() : undefined}
            />
            <Address
              label="IPv4"
              value={node.addresses.ipv4}
              empty={m.nodes_address_none()}
            />
            <Address
              label="IPv6"
              value={node.addresses.ipv6}
              empty={m.nodes_address_none()}
            />
          </FactList>
        </Section>
        <Section title={m.nodes_runtime()}>
          <FactList>
            <FactRow
              label={m.nodes_connected_peers()}
              detail={
                otherStates.length
                  ? otherStates
                      .map(
                        ({ value, count }) =>
                          `${connectionStatusLabel[value]()} ${count}`
                      )
                      .join(" · ")
                  : undefined
              }
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
              <span className="tabular-nums">
                {view?.freshness === "current" ? connected : "—"}
              </span>
            </FactRow>
            <FactRow label={m.nodes_configuration()}>
              <span
                className={cn(
                  attention && "text-warning",
                  sync.kind === "failed" && "text-destructive"
                )}
              >
                {sync.kind === "current" &&
                ((targetApplied && !current) || node.desiredExecution === null)
                  ? m.nodes_sync_applied({ revision: sync.revision })
                  : syncText(sync, current)}
              </span>
            </FactRow>
          </FactList>
        </Section>
      </>

      <Section title={m.nodes_network_location()}>
        {ips.length ? (
          <FactList>
            {ips.map((ip, index) => {
              const geo = geoOf(ip)
              const previous = index > 0 ? geoOf(ips[index - 1] ?? "") : null
              const metadata = geo
                ? [geoText(geo), geo.kind === "found" ? asnText(geo) : null]
                    .filter(Boolean)
                    .join(" · ")
                : "—"
              const repeated =
                geo?.kind === "found" &&
                previous?.kind === "found" &&
                geoText(geo) === geoText(previous) &&
                asnText(geo) === asnText(previous)
              return (
                <FactRow
                  key={ip}
                  label={m.nodes_public_address()}
                  detail={
                    repeated ? undefined : (
                      <span title={metadata}>{metadata}</span>
                    )
                  }
                  action={
                    <CopyButton value={ip} label={m.nodes_public_address()} />
                  }
                >
                  <span className="font-mono" title={ip}>
                    {ip}
                  </span>
                </FactRow>
              )
            })}
          </FactList>
        ) : (
          <FactList>
            <FactRow label={m.nodes_public_address()}>
              {addresses?.locationAbsent
                ? locationAbsentLabel[addresses.locationAbsent]()
                : m.nodes_location_none()}
            </FactRow>
          </FactList>
        )}
      </Section>

      {report && report.problems.length > 0 ? (
        <Section title={m.nodes_problems()} count={report.problems.length}>
          <ul className="flex flex-col gap-2">
            {report.problems.map((problem, index) => (
              <li
                key={index}
                className="flex min-h-12 items-center justify-between gap-3 rounded-tile bg-fill-subtle px-3.5 py-2 text-row"
              >
                <span className="text-destructive">{problemText(problem)}</span>
                <span className="shrink-0 text-meta text-muted-foreground">
                  {agoTime(problem.observedAt, now)}
                </span>
              </li>
            ))}
          </ul>
        </Section>
      ) : null}
    </div>
  )
}

function Address({
  label,
  value,
  note,
  empty = "—",
}: {
  label: string
  value: string | null
  note?: string
  empty?: string
}) {
  return (
    <FactRow
      label={label}
      detail={note}
      action={value ? <CopyButton value={value} label={label} /> : null}
    >
      <span className="font-mono" title={value ?? undefined}>
        {value ?? empty}
      </span>
    </FactRow>
  )
}
