import type { AggregatedLink } from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useNavigate } from "@tanstack/react-router"
import { PencilIcon } from "lucide-react"
import { useMemo, useState } from "react"

import { failureText } from "@/api/failure"
import { bucketsOf, useHistory } from "@/api/queries/use-history"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { DIALER_PROTOCOLS } from "@/lib/configuration/dialer-model"
import {
  edgeDirections,
  type DialDirection,
} from "@/lib/configuration/topology-view"
import { agoTime, dateTime } from "@/lib/format"
import { millis } from "@/lib/history/format"
import { bucketSeries } from "@/lib/history/series"
import { historyWindow } from "@/lib/history/window"
import {
  evidenceLabel,
  linkPhaseLabel,
  transportText,
} from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { useNetworkScene } from "../-shell/network-scene"
import { linkSummary, orderText } from "./link-text"
import {
  Fact,
  Facts,
  LevelBody,
  LevelHead,
  LinkSwatch,
  PanelSection,
} from "./panel-kit"
import type { LinkEnds, MapSelection } from "./selection"

const pairKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

/**
 * One link between two nodes: what they report about it now, how fast the
 * probes between them were in the last hour, what each may dial the other
 * with, and the topology rule it rests on.
 */
export function LinkLevel({
  networkId,
  ends,
  selection,
}: {
  networkId: string
  ends: LinkEnds
  selection: MapSelection
}) {
  const scene = useNetworkScene()
  const navigate = useNavigate()
  const [a, b] = ends
  const key = pairKey(a, b)
  const edge =
    scene.edges.find((item) => item.key === key) ??
    scene.connections.find((item) => item.key === key) ??
    null
  const config = scene.configuration?.configuration
  const declaration = edge?.declaration
  const channels = (edge?.channels ?? []).filter(
    (channel) => channel.phase !== "closed"
  )

  return (
    <>
      <LevelHead
        title={
          <>
            {scene.nameOf(a)} ↔ {scene.nameOf(b)}
          </>
        }
        onBack={selection.back}
        onClose={selection.close}
        meta={
          scene.stateLoading ? (
            <Skeleton className="h-3 w-32 rounded-full" />
          ) : (
            <>
              <LinkSwatch look={edge?.look ?? "allowed"} />
              <span className="truncate">{linkSummary(edge)}</span>
            </>
          )
        }
      />
      <LevelBody>
        <PanelSection title={m.map_link_runtime()}>
          {scene.stateLoading ? (
            <div className="flex flex-col gap-2 px-4 py-1">
              <Skeleton className="h-3 w-40 rounded-full" />
              <Skeleton className="h-3 w-28 rounded-full" />
            </div>
          ) : scene.state.isError && !scene.state.data ? (
            <p className="px-4 text-row text-muted-foreground">
              {failureText(scene.state.error) ?? m.nodes_state_failed()}
            </p>
          ) : channels.length === 0 ? (
            <p className="px-4 text-row text-muted-foreground">
              {m.map_link_no_channel()}
            </p>
          ) : (
            channels.map((channel) => (
              <Channel
                key={channel.linkId}
                channel={channel}
                nameOf={scene.nameOf}
              />
            ))
          )}
        </PanelSection>

        <PanelSection title={m.map_link_latency()}>
          <Latency networkId={networkId} a={a} b={b} nameOf={scene.nameOf} />
        </PanelSection>

        {scene.canRead && !config ? (
          <PanelSection title={m.map_link_configuration()}>
            {scene.configurationError ? (
              <p className="px-4 text-row text-muted-foreground">
                {failureText(scene.configurationError) ??
                  m.nodes_listeners_failed()}
              </p>
            ) : (
              <div className="flex flex-col gap-2 px-4 py-1">
                <Skeleton className="h-3 w-44 rounded-full" />
                <Skeleton className="h-3 w-32 rounded-full" />
              </div>
            )}
          </PanelSection>
        ) : null}
        {config ? (
          <PanelSection title={m.map_link_capability()}>
            <Facts>
              {edgeDirections(config, {
                a,
                b,
                transports: declaration?.transports ?? null,
              }).map((direction) => (
                <Fact
                  key={direction.from}
                  stacked
                  label={m.map_link_dials({
                    from: scene.nameOf(direction.from),
                    to: scene.nameOf(direction.to),
                  })}
                >
                  <DirectionText direction={direction} nameOf={scene.nameOf} />
                </Fact>
              ))}
            </Facts>
            <p className="px-4 text-meta text-muted-foreground">
              {m.map_link_capability_note()}
            </p>
          </PanelSection>
        ) : null}

        {config ? (
          <PanelSection title={m.map_link_configuration()}>
            <Facts>
              <Fact label={m.map_link_rule()}>
                {declaration === undefined
                  ? m.map_link_not_declared()
                  : declaration.rule === null
                    ? m.map_link_rule_default()
                    : m.overview_topology_rule({
                        number: declaration.rule + 1,
                      })}
              </Fact>
              {declaration ? (
                <Fact
                  label={m.configuration_topology_transports()}
                  detail={
                    declaration.transports === null
                      ? m.configuration_topology_transports_any()
                      : undefined
                  }
                >
                  {declaration.transports?.length === 0
                    ? m.map_link_no_carrier()
                    : orderText(declaration.transports ?? DIALER_PROTOCOLS)}
                </Fact>
              ) : null}
            </Facts>
            <div className="px-4 pt-1">
              <Button
                size="sm"
                variant="outline"
                className="rounded-full"
                onClick={() =>
                  void navigate({
                    to: "/networks/$network/configuration/$view",
                    params: { network: networkId, view: "topology" },
                  })
                }
              >
                <PencilIcon />
                {m.map_link_edit()}
              </Button>
            </div>
          </PanelSection>
        ) : null}
      </LevelBody>
    </>
  )
}

function Channel({
  channel,
  nameOf,
}: {
  channel: AggregatedLink
  nameOf: (id: string) => string
}) {
  const now = useNow(5_000)
  return (
    <Facts>
      <Fact label={m.map_link_carrier()}>
        {transportText(channel.transport)}
      </Fact>
      <Fact
        label={m.map_link_phase()}
        detail={evidenceLabel[channel.evidence]()}
      >
        {linkPhaseLabel[channel.phase]()}
      </Fact>
      <Fact label={m.map_link_initiator()}>
        {nameOf(channel.initiatorNodeId)}
      </Fact>
      {channel.since ? (
        <Fact label={m.map_link_since()}>
          <span title={dateTime(channel.since) ?? undefined}>
            {agoTime(channel.since, now) ?? "—"}
          </span>
        </Fact>
      ) : null}
      {channel.inUse != null ? (
        <Fact label={m.map_link_use()}>
          {channel.inUse ? m.map_link_in_use() : m.map_link_unused()}
        </Fact>
      ) : null}
    </Facts>
  )
}

function DirectionText({
  direction,
  nameOf,
}: {
  direction: DialDirection
  nameOf: (id: string) => string
}) {
  if (direction.transports.length > 0)
    return <>{orderText(direction.transports)}</>
  return (
    <span className="text-muted-foreground">
      {direction.missing === "edge"
        ? m.map_link_no_carrier()
        : direction.missing === "dialer"
          ? m.map_link_no_dialer({ node: nameOf(direction.from) })
          : m.map_link_no_listener({ node: nameOf(direction.to) })}
    </span>
  )
}

/** The last hour of direct probes each way, as the controller recorded them. */
function Latency({
  networkId,
  a,
  b,
  nameOf,
}: {
  networkId: string
  a: string
  b: string
  nameOf: (id: string) => string
}) {
  const [window] = useState(() => historyWindow("1h", Date.now()))
  return (
    <Facts>
      {[
        [a, b],
        [b, a],
      ].map(([from = a, to = b]) => (
        <Direction
          key={from}
          networkId={networkId}
          window={window}
          from={from}
          to={to}
          label={`${nameOf(from)} → ${nameOf(to)}`}
        />
      ))}
    </Facts>
  )
}

function Direction({
  networkId,
  window,
  from,
  to,
  label,
}: {
  networkId: string
  window: ReturnType<typeof historyWindow>
  from: string
  to: string
  label: string
}) {
  const filter = useMemo(
    () => ({
      mode: "buckets" as const,
      kinds: ["probe_completed" as const],
      sourceNodeId: from,
      peerNodeId: to,
    }),
    [from, to]
  )
  const history = useHistory(networkId, window, filter)
  const series = useMemo(
    () =>
      bucketSeries(bucketsOf(history.pages), window).filter(
        (item) =>
          item.path.length === 2 &&
          (item.kind === "neighbor" || item.kind === "path")
      ),
    [history.pages, window]
  )
  const samples = series.reduce((sum, item) => sum + item.totals.samples, 0)
  const success = series.reduce((sum, item) => sum + item.totals.success, 0)
  const mean =
    success > 0
      ? series.reduce(
          (sum, item) => sum + (item.totals.meanUs ?? 0) * item.totals.success,
          0
        ) / success
      : null
  return (
    <Fact
      stacked
      label={label}
      detail={
        samples > 0
          ? m.map_link_samples({
              success: String(success),
              samples: String(samples),
            })
          : undefined
      }
    >
      {history.query.data === undefined ? (
        history.query.isError ? (
          <span className="text-muted-foreground">{m.history_failed()}</span>
        ) : (
          <Skeleton className="h-3 w-20 rounded-full" />
        )
      ) : mean !== null ? (
        millis(mean)
      ) : (
        <span className="text-muted-foreground">
          {samples > 0 ? m.history_no_success() : m.map_link_no_probes()}
        </span>
      )}
    </Fact>
  )
}
