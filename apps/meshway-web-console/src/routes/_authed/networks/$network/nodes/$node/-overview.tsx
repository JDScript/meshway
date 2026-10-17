import type {
  NetworkSummary,
  NodeInventoryView,
  StateView,
} from "@meshway/control-client"
import { cn } from "@meshway/ui/cn"
import type { UseQueryResult } from "@tanstack/react-query"
import { CircleAlertIcon, CircleCheckIcon } from "lucide-react"

import { failureText } from "@/api/failure"
import { useAddressGeo } from "@/api/queries/ip-geo"
import { CopyButton } from "@/components/chrome/copy-button"
import { EmptyLine } from "@/components/chrome/section"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { agoTime, dateTime } from "@/lib/format"
import { asnText, geoText, NO_ADDRESSES, nodeAddresses } from "@/lib/geo"
import { millis } from "@/lib/history/format"
import { nodeConnections } from "@/lib/nodes/connections"
import {
  configurationSync,
  fullReport,
  isPast,
  stateOf,
} from "@/lib/nodes/state-model"
import {
  exitDnsText,
  exitStatusText,
  locationAbsentLabel,
  netmapText,
  problemText,
} from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import type { NodeTab } from "../-search"
import { useNodeConfiguration } from "./-draft"
import { Card, LoadingRow, Row, Rows, SummaryTile } from "./-kit"
import { useNeighborLatency } from "./-latency/neighbor-latency"

/**
 * The node at a glance: four tiles that sum up its connections, latency,
 * configuration and place (each opening its tab), how to reach it, where
 * it is on the internet, and anything it reports needing attention.
 */
export function NodeOverview({
  node,
  state,
  onTab,
  networkId,
  network,
  nameOf,
}: {
  node: NodeInventoryView
  state: UseQueryResult<StateView>
  onTab: (tab: NodeTab) => void
  networkId: string
  network: NetworkSummary | null
  nameOf: (id: string) => string
}) {
  const now = useNow(1_000)
  const view = stateOf(state.data, node.id)
  const report = fullReport(view)
  const online = view?.freshness === "current"
  const stateLoading = state.data === undefined && !state.isError
  const stateFailed = state.data === undefined && state.isError
  const stateFailure = failureText(state.error) ?? m.nodes_state_failed()
  const sync = configurationSync(report?.configuration ?? null, node.netmap)
  const connections = nodeConnections(state.data?.links ?? [], node.id)
  const connected = connections.filter(
    (connection) => connection.status === "connected"
  )
  const latency = useNeighborLatency(networkId, node.id)
  const fastest = [...latency.byPeer].flatMap(([peer, value]) =>
    value.meanUs === null ? [] : [{ peer, meanUs: value.meanUs }]
  )
  fastest.sort((a, b) => a.meanUs - b.meanUs)
  const best = fastest[0]

  const addresses = view ? nodeAddresses(view.derived) : null
  const geoOf = useAddressGeo(addresses?.scopes ?? NO_ADDRESSES)
  const ips = [...(addresses?.scopes ?? NO_ADDRESSES)]
    .filter(([, scope]) => scope === "public")
    .map(([ip]) => ip)
    .sort()
  const placeGeo = addresses?.location
    ? geoOf(addresses.location.address)
    : null

  const configuration = useNodeConfiguration(networkId, node.id, network).state
  const config =
    configuration.kind === "manager" ? configuration.view.configuration : null
  const configuredName = config?.nodes[node.id]?.hostname
  const dnsName =
    configuredName && config ? `${configuredName}.${config.dns.domain}` : null

  // Offline, it runs what it last took until the lease the server signed ends.
  const lease = node.lease
  const leaseDetail =
    view && !online && lease
      ? (isPast(lease.expiresAt, now)
          ? m.nodes_tile_config_lease_over
          : m.nodes_tile_config_lease)({
          at: dateTime(lease.expiresAt) ?? lease.expiresAt,
          netmap: netmapText(lease.netmap),
        })
      : null

  // What needs someone: the node's own problems, then what its exit and
  // its Server key say, and a netmap the server will not send.
  const exit = report?.exitCapability
  const exitDns = report?.exitDns
  const alerts: {
    key: string
    text: string
    at?: string
    tone: "destructive" | "warning"
  }[] = [
    ...(report?.problems ?? []).map((problem, index) => ({
      key: `problem:${index}`,
      text: problemText(problem),
      at: problem.observedAt,
      tone: "destructive" as const,
    })),
    ...(exit?.families ?? [])
      .filter((entry) => entry.status.status === "unavailable")
      .map((entry) => ({
        key: `exit:${entry.family}`,
        text: m.nodes_alert_exit({ reason: exitStatusText(entry.status) }),
        at: exit?.observedAt,
        tone: "warning" as const,
      })),
    ...(exitDns?.status.status === "unavailable"
      ? [
          {
            key: "exit-dns",
            text: m.nodes_alert_exit_dns({
              reason: exitDnsText(exitDns.status),
            }),
            tone: "warning" as const,
          },
        ]
      : []),
    ...(node.serverKey?.status === "keyMismatch"
      ? [
          {
            key: "server-key",
            text: m.nodes_alert_server_key(),
            tone: "destructive" as const,
          },
        ]
      : []),
    ...(node.netmapUnavailable?.reason === "limit"
      ? [
          {
            key: "netmap-limit",
            text: m.nodes_alert_netmap_limit(),
            tone: "destructive" as const,
          },
        ]
      : []),
  ]

  const syncTile = (() => {
    switch (sync.kind) {
      case "current":
        return {
          value: m.nodes_tile_config_applied(),
          detail: m.nodes_tile_config_latest({
            netmap: netmapText(sync.applied),
          }),
          tone: "online" as const,
        }
      case "applied":
        return {
          value: m.nodes_tile_config_applied(),
          detail: m.nodes_tile_config_reported({
            netmap: netmapText(sync.applied),
          }),
          tone: undefined,
        }
      case "applying":
        return {
          value: m.nodes_tile_config_in_progress(),
          detail: m.nodes_tile_config_revision({
            netmap: netmapText(sync.target),
          }),
          tone: "attention" as const,
        }
      case "failed":
        return {
          value: m.nodes_tile_config_failed(),
          detail: m.nodes_sync_failed({ netmap: netmapText(sync.target) }),
          tone: "offline" as const,
        }
      case "behind":
        return {
          // Online, it takes the new one on its next sync; offline, it waits.
          value: online
            ? m.nodes_tile_config_syncing()
            : m.nodes_tile_config_behind(),
          detail: m.nodes_sync_behind({
            applied: netmapText(sync.applied),
            assigned: netmapText(sync.assigned),
          }),
          tone: "attention" as const,
        }
      case "none":
        return {
          value: m.nodes_tile_config_none(),
          detail: m.nodes_sync_none(),
          tone: "attention" as const,
        }
      case "unknown":
        return {
          value: "—",
          detail: m.nodes_tile_unreported(),
          tone: "muted" as const,
        }
    }
  })()

  return (
    <div className="@container flex flex-col gap-7">
      {state.isError && state.data !== undefined ? (
        <EmptyLine>{m.nodes_state_stale({ reason: stateFailure })}</EmptyLine>
      ) : null}

      <div className="grid grid-cols-2 gap-3 @3xl:grid-cols-4">
        <SummaryTile
          label={m.nodes_tile_connections()}
          loading={stateLoading}
          value={
            online
              ? connections.length > connected.length
                ? `${connected.length} / ${connections.length}`
                : String(connected.length)
              : "—"
          }
          tone={online ? undefined : "muted"}
          detail={
            stateFailed
              ? stateFailure
              : !view
                ? m.nodes_tile_unreported()
                : !online
                  ? m.nodes_tile_connections_offline()
                  : connected.length
                    ? connected
                        .slice(0, 2)
                        .map((connection) => nameOf(connection.peerId))
                        .join("、") +
                      (connected.length > 2
                        ? m.nodes_tile_more({ count: connected.length - 2 })
                        : "")
                    : m.nodes_tile_connections_none()
          }
          onClick={() => onTab("connections")}
        />
        <SummaryTile
          label={m.nodes_tile_latency()}
          loading={latency.loading}
          value={best ? millis(best.meanUs) : "—"}
          tone={best ? undefined : "muted"}
          detail={
            best
              ? m.nodes_tile_latency_to({ peer: nameOf(best.peer) })
              : latency.failed
                ? m.nodes_tile_latency_failed()
                : m.nodes_tile_latency_none()
          }
          onClick={() => onTab("diagnostics")}
        />
        <SummaryTile
          label={m.nodes_configuration()}
          loading={stateLoading}
          value={syncTile.value}
          tone={syncTile.tone}
          detail={stateFailed ? stateFailure : (leaseDetail ?? syncTile.detail)}
          onClick={() => onTab("diagnostics")}
        />
        <SummaryTile
          label={m.nodes_tile_place()}
          loading={stateLoading || placeGeo?.kind === "pending"}
          value={
            placeGeo?.kind === "found"
              ? (placeGeo.geo.city ?? placeGeo.geo.region ?? geoText(placeGeo))
              : "—"
          }
          tone={placeGeo?.kind === "found" ? undefined : "muted"}
          detail={
            stateFailed
              ? stateFailure
              : placeGeo?.kind === "found"
                ? asnText(placeGeo)
                : placeGeo
                  ? geoText(placeGeo)
                  : addresses?.locationAbsent
                    ? locationAbsentLabel[addresses.locationAbsent]()
                    : m.nodes_location_none()
          }
        />
      </div>

      <Card title={m.nodes_mesh_access()}>
        <Rows>
          {configuration.kind === "loading" &&
          network?.actions.readConfiguration ? (
            <LoadingRow label={m.nodes_dns_name()} />
          ) : dnsName ? (
            <Row
              label={m.nodes_dns_name()}
              mono
              detail={
                config?.dns.magicDNS === false
                  ? m.nodes_dns_disabled()
                  : undefined
              }
              action={<CopyButton value={dnsName} label={m.nodes_dns_name()} />}
            >
              <span title={dnsName}>{dnsName}</span>
            </Row>
          ) : null}
          {(["ipv4", "ipv6"] as const).map((family) => {
            const value = node.addresses[family]
            return (
              <Row
                key={family}
                label={family === "ipv4" ? "IPv4" : "IPv6"}
                mono={value !== null}
                tone={value === null ? "muted" : undefined}
                action={
                  value ? (
                    <CopyButton
                      value={value}
                      label={family === "ipv4" ? "IPv4" : "IPv6"}
                    />
                  ) : null
                }
              >
                {value ?? m.nodes_address_none()}
              </Row>
            )
          })}
        </Rows>
      </Card>

      <Card title={m.nodes_network_location()}>
        <Rows>
          {stateLoading ? (
            <LoadingRow label={m.nodes_public_address()} />
          ) : ips.length ? (
            ips.map((ip) => {
              const geo = geoOf(ip)
              return (
                <Row
                  key={ip}
                  label={m.nodes_public_address()}
                  mono
                  detail={
                    geo
                      ? [
                          geoText(geo),
                          geo.kind === "found" ? asnText(geo) : null,
                        ]
                          .filter(Boolean)
                          .join(" · ")
                      : undefined
                  }
                  action={
                    <CopyButton value={ip} label={m.nodes_public_address()} />
                  }
                >
                  {ip}
                </Row>
              )
            })
          ) : (
            <Row label={m.nodes_public_address()} tone="muted">
              {stateFailed
                ? stateFailure
                : !view
                  ? m.nodes_tile_unreported()
                  : addresses?.locationAbsent
                    ? locationAbsentLabel[addresses.locationAbsent]()
                    : m.nodes_location_none()}
            </Row>
          )}
        </Rows>
      </Card>

      <Card title={m.nodes_attention()} aside={alerts.length || undefined}>
        {alerts.length > 0 ? (
          <Rows>
            {alerts.map((alert) => (
              <div
                key={alert.key}
                className="flex min-h-11 items-center gap-3 px-4 py-2.5"
              >
                <CircleAlertIcon
                  className={cn(
                    "size-4 shrink-0",
                    alert.tone === "warning"
                      ? "text-warning"
                      : "text-destructive"
                  )}
                />
                <span className="min-w-0 flex-1 text-row">{alert.text}</span>
                {alert.at ? (
                  <span className="shrink-0 text-meta text-muted-foreground">
                    {agoTime(alert.at, now)}
                  </span>
                ) : null}
              </div>
            ))}
          </Rows>
        ) : (
          <div className="flex min-h-11 items-center gap-3 rounded-tile bg-fill-subtle px-4 py-2.5 text-row">
            {stateLoading ? (
              <Skeleton className="h-3 w-44 rounded-full" />
            ) : stateFailed ? (
              <span className="text-muted-foreground">{stateFailure}</span>
            ) : report ? (
              <>
                <CircleCheckIcon className="size-4 shrink-0 text-success" />
                {m.nodes_attention_none()}
              </>
            ) : (
              <span className="text-muted-foreground">
                {m.nodes_attention_unreported()}
              </span>
            )}
          </div>
        )}
      </Card>
    </div>
  )
}
