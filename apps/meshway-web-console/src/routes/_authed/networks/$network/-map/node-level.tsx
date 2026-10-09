import { Button } from "@meshway/ui/button"
import { ArrowUpRightIcon } from "lucide-react"

import { failureText } from "@/api/failure"
import { CopyButton } from "@/components/chrome/copy-button"
import { ListNote, ListPlaceholders, ListRow } from "@/components/chrome/list"
import { NodeStatusDot } from "@/components/chrome/node-status"
import { Skeleton } from "@/components/ui/skeleton"
import {
  TARGET_PROBLEM,
  UNAVAILABLE_REASON,
} from "@/lib/configuration/evaluation"
import { asnText, placeText } from "@/lib/geo"
import type { MapConnection, MapNode } from "@/lib/network-map"
import { hopCount, viaOf, type NodeRoute } from "@/lib/network-routes"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { useNetworkScene } from "../-shell/network-scene"
import { useNodeNavigation } from "../nodes/-navigation"
import { linkStateText, linkSummary } from "./link-text"
import { LevelBody, LevelHead, LinkSwatch, PanelSection } from "./panel-kit"
import type { MapSelection } from "./selection"

const pairKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

/** Planned paths first, shortest first; then those without one; then those still asked. */
const routeRank = (route: NodeRoute) =>
  route.status === "pending"
    ? 1000
    : route.status === "failed"
      ? 999
      : (hopCount(route) ?? 998)

/**
 * One node: its status and address, the way into its details, and how it
 * reaches every other node — each planned path's hops, chosen to see that
 * link. A member, who cannot read plans, gets the node's current links.
 */
export function NodeLevel({
  networkId,
  node,
  selection,
}: {
  networkId: string
  node: MapNode
  selection: MapSelection
}) {
  const scene = useNetworkScene()
  const navigation = useNodeNavigation(networkId)
  const listed = scene.inventory?.find((item) => item.id === node.id)
  const address = listed?.addresses.ipv4 ?? listed?.addresses.ipv6 ?? null
  const place =
    node.geo?.kind === "found"
      ? [placeText(node.geo.geo), asnText(node.geo)].filter(Boolean).join(" · ")
      : null
  const byPair = new Map(scene.edges.map((edge) => [edge.key, edge]))
  const connectionOf = (a: string, b: string) =>
    byPair.get(pairKey(a, b)) ??
    scene.connections.find((edge) => edge.key === pairKey(a, b)) ??
    null
  const statusOf = (id: string) =>
    scene.nodes.find((item) => item.id === id)?.status ?? "unknown"

  return (
    <>
      <LevelHead
        title={node.name}
        onBack={selection.group ? selection.back : undefined}
        onClose={selection.close}
        meta={
          <>
            {scene.stateLoading ? (
              <Skeleton className="h-3 w-12 rounded-full" />
            ) : (
              <>
                <NodeStatusDot status={node.status} size="sm" />
                <span className="shrink-0">{statusLabel[node.status]()}</span>
              </>
            )}
            {address ? (
              <>
                <span aria-hidden="true">·</span>
                <span className="min-w-0 truncate font-mono">{address}</span>
                <CopyButton value={address} label={m.nodes_mesh_access()} />
              </>
            ) : null}
          </>
        }
      />
      <div className="flex shrink-0 items-center gap-3 px-4 pb-3">
        <span className="min-w-0 flex-1 truncate text-meta text-muted-foreground">
          {place}
        </span>
        <Button
          size="sm"
          variant="outline"
          className="shrink-0 rounded-full"
          onClick={() => navigation.openNode(node.id)}
        >
          {m.map_open_node()}
          <ArrowUpRightIcon />
        </Button>
      </div>
      <LevelBody>
        {scene.canRead && !scene.configuration ? (
          <PanelSection title={m.map_routes()}>
            {scene.configurationError ? (
              <p className="px-4 text-row text-muted-foreground">
                {failureText(scene.configurationError) ??
                  m.nodes_listeners_failed()}
              </p>
            ) : (
              <ul className="flex flex-col gap-0.5 px-2">
                <ListPlaceholders rows={3} />
              </ul>
            )}
          </PanelSection>
        ) : scene.routes ? (
          <PanelSection
            title={m.map_routes()}
            aside={scene.routes.length || undefined}
          >
            <ul className="flex flex-col gap-0.5 px-2">
              {scene.routes.length === 0 ? (
                <ListNote>{m.map_routes_none()}</ListNote>
              ) : (
                [...scene.routes]
                  .sort(
                    (x, y) =>
                      routeRank(x) - routeRank(y) ||
                      scene
                        .nameOf(x.target)
                        .localeCompare(scene.nameOf(y.target))
                  )
                  .map((route) => (
                    <RouteRow
                      key={route.target}
                      route={route}
                      name={scene.nameOf(route.target)}
                      status={statusOf(route.target)}
                      nameOf={scene.nameOf}
                      connectionOf={connectionOf}
                      selectedPair={
                        selection.link
                          ? pairKey(selection.link[0], selection.link[1])
                          : null
                      }
                      onLink={selection.selectLink}
                    />
                  ))
              )}
            </ul>
          </PanelSection>
        ) : (
          <PanelSection title={m.map_neighbours()}>
            <ul className="flex flex-col gap-0.5 px-2">
              {scene.stateLoading ? (
                <ListPlaceholders rows={2} />
              ) : scene.state.isError && !scene.state.data ? (
                <ListNote>
                  {failureText(scene.state.error) ?? m.nodes_state_failed()}
                </ListNote>
              ) : (
                <Neighbours
                  node={node}
                  connections={scene.connections}
                  nameOf={scene.nameOf}
                  onLink={selection.selectLink}
                />
              )}
            </ul>
          </PanelSection>
        )}
      </LevelBody>
    </>
  )
}

function routeText(route: NodeRoute, nameOf: (id: string) => string) {
  switch (route.status) {
    case "pending":
      return null
    case "failed":
      return failureText(route.error) ?? m.map_route_failed()
    case "planned": {
      if (route.hops.length === 0)
        return route.problem
          ? TARGET_PROBLEM[route.problem]()
          : route.outcome.type === "unavailable"
            ? UNAVAILABLE_REASON[route.outcome.reason]()
            : m.map_route_none()
      const via = viaOf(route)
      return [
        via.length === 0
          ? m.map_route_direct()
          : m.map_route_via({ nodes: via.map(nameOf).join(" → ") }),
        route.authorized ? null : m.map_route_unauthorized(),
      ]
        .filter(Boolean)
        .join(" · ")
    }
  }
}

function RouteRow({
  route,
  name,
  status,
  nameOf,
  connectionOf,
  selectedPair,
  onLink,
}: {
  route: NodeRoute
  name: string
  status: MapNode["status"]
  nameOf: (id: string) => string
  connectionOf: (a: string, b: string) => MapConnection | null
  selectedPair: string | null
  onLink: (a: string, b: string) => void
}) {
  const text = routeText(route, nameOf)
  const hops = route.status === "planned" ? route.hops : []
  return (
    <li
      data-unauthorized={route.status === "planned" && !route.authorized}
      className="flex flex-col gap-1.5 rounded-row px-3 py-2.5"
    >
      <div className="flex h-5 min-w-0 items-center gap-3">
        <span className="flex size-4 shrink-0 items-center justify-center">
          <NodeStatusDot status={status} />
        </span>
        <span className="min-w-0 flex-1 truncate text-row font-medium">
          {name}
        </span>
        {text === null ? (
          <Skeleton className="h-3 w-16 rounded-full" />
        ) : (
          <span className="max-w-[55%] shrink-0 truncate text-meta text-muted-foreground">
            {text}
          </span>
        )}
      </div>
      {hops.length > 0 ? (
        <div className="flex flex-wrap gap-1 pl-7">
          {hops.map((hop) => {
            const connection = connectionOf(hop.from, hop.to)
            return (
              <button
                key={`${hop.from}>${hop.to}`}
                type="button"
                aria-pressed={pairKey(hop.from, hop.to) === selectedPair}
                title={linkSummary(connection)}
                className="map-hop"
                onClick={() => onLink(hop.from, hop.to)}
              >
                <LinkSwatch look={connection?.look ?? "allowed"} />
                <span className="truncate">
                  {hops.length > 1 ? `${nameOf(hop.to)} · ` : null}
                  {linkStateText(connection)}
                </span>
              </button>
            )
          })}
        </div>
      ) : null}
    </li>
  )
}

function Neighbours({
  node,
  connections,
  nameOf,
  onLink,
}: {
  node: MapNode
  connections: readonly MapConnection[]
  nameOf: (id: string) => string
  onLink: (a: string, b: string) => void
}) {
  const own = connections.filter(
    (edge) =>
      edge.status !== "history" && (edge.a === node.id || edge.b === node.id)
  )
  if (own.length === 0) return <ListNote>{m.nodes_links_none()}</ListNote>
  return own.map((edge) => {
    const peer = edge.a === node.id ? edge.b : edge.a
    return (
      <ListRow
        key={edge.key}
        lead={<LinkSwatch look={edge.look} />}
        title={<span className="truncate">{nameOf(peer)}</span>}
        subtitle={<span className="truncate">{linkSummary(edge)}</span>}
        onClick={() => onLink(node.id, peer)}
      />
    )
  })
}
