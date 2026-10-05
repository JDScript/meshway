import type { AggregatedLink } from "@meshway/control-client"

import { NodeStatusDot } from "@/components/chrome/node-status"
import { dateTime } from "@/lib/format"
import { LOCATION_SOURCE, placeText } from "@/lib/geo"
import type { MapNode } from "@/lib/network-map"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

/**
 * A located node pointed at on the globe: where the controller's GeoIP
 * places it and on what that rests (the endpoint, its source and validity,
 * the dataset). Clicking the node opens it.
 */
export function NodeCard({ node }: { node: MapNode }) {
  const location = node.location
  const position = location?.data.position
  return (
    <div className="flex flex-col gap-3 p-4">
      <div className="flex min-w-0 items-center gap-2">
        <NodeStatusDot status={node.status} />
        <span className="min-w-0 flex-1 truncate text-row font-medium">
          {node.name}
        </span>
        <span className="shrink-0 text-meta text-muted-foreground">
          {statusLabel[node.status]()}
        </span>
      </div>
      {location ? (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-meta">
          <dt className="text-muted-foreground">
            {m.overview_location_approximate()}
          </dt>
          <dd className="min-w-0 truncate">
            {(location && placeText(location.data)) ?? "—"}
            {position?.accuracyRadiusKm != null ? (
              <span className="text-muted-foreground">
                {" "}
                ·{" "}
                {m.overview_location_accuracy({
                  km: String(Math.round(position.accuracyRadiusKm)),
                })}
              </span>
            ) : null}
          </dd>
          <dt className="text-muted-foreground">
            {m.overview_location_address()}
          </dt>
          <dd className="min-w-0 truncate font-mono">
            {location.address}
            <span className="font-sans text-muted-foreground">
              {" "}
              · {LOCATION_SOURCE[location.endpointSource]()}
            </span>
          </dd>
          <dt className="text-muted-foreground">
            {m.overview_location_observed()}
          </dt>
          <dd className="tabular-nums">{dateTime(location.observedAt)}</dd>
          <dt className="text-muted-foreground">
            {m.overview_location_expires()}
          </dt>
          <dd className="tabular-nums">{dateTime(location.expiresAt)}</dd>
          <dt className="text-muted-foreground">
            {m.overview_location_dataset()}
          </dt>
          <dd className="min-w-0 truncate font-mono">{location.datasetId}</dd>
        </dl>
      ) : null}
    </div>
  )
}

/** The one value every node gives, or null when they differ. */
const shared = <T,>(values: readonly T[]): T | null =>
  values.length > 0 && values.every((value) => value === values[0])
    ? (values[0] ?? null)
    : null

/**
 * Nodes pointed at together: too close on the globe to tell apart. Each
 * with its status; their place and address once when they share it, else
 * by node; and the current links between them, which the globe cannot draw.
 */
export function GroupCard({
  nodes,
  links,
}: {
  nodes: readonly MapNode[]
  links: readonly AggregatedLink[]
}) {
  const ids = new Set(nodes.map((node) => node.id))
  const between = links.filter((link) => ids.has(link.a) && ids.has(link.b))
  const place = (node: MapNode) =>
    (node.location && placeText(node.location.data)) ?? "—"
  const address = (node: MapNode) =>
    node.location
      ? `${node.location.address} · ${LOCATION_SOURCE[node.location.endpointSource]()}`
      : "—"
  const commonPlace = shared(nodes.map(place))
  const commonAddress = shared(nodes.map(address))
  return (
    <div className="flex flex-col gap-3 p-4">
      <div className="flex min-w-0 items-baseline gap-2">
        <span className="shrink-0 text-row font-medium">
          {m.overview_group_nodes({ count: nodes.length })}
        </span>
        {commonPlace !== null ? (
          <span className="min-w-0 flex-1 truncate text-right text-meta text-muted-foreground">
            {commonPlace}
          </span>
        ) : null}
      </div>
      <ul className="flex flex-col gap-1.5">
        {nodes.map((node) => (
          <li key={node.id} className="flex min-w-0 flex-col">
            <div className="flex min-w-0 items-center gap-2">
              <NodeStatusDot status={node.status} />
              <span className="min-w-0 flex-1 truncate text-row">
                {node.name}
              </span>
              <span className="shrink-0 text-meta text-muted-foreground">
                {statusLabel[node.status]()}
              </span>
            </div>
            {commonPlace === null || commonAddress === null ? (
              <span className="truncate pl-4 text-meta text-muted-foreground">
                {[
                  commonPlace === null ? place(node) : null,
                  commonAddress === null ? address(node) : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            ) : null}
          </li>
        ))}
      </ul>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-meta">
        {commonAddress !== null ? (
          <>
            <dt className="text-muted-foreground">
              {m.overview_location_address()}
            </dt>
            <dd className="min-w-0 truncate">{commonAddress}</dd>
          </>
        ) : null}
        <dt className="text-muted-foreground">{m.overview_group_between()}</dt>
        <dd>
          {between.length === 0
            ? m.overview_group_links_none()
            : m.overview_group_links({ count: between.length })}
        </dd>
      </dl>
    </div>
  )
}
