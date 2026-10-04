import { StatusDot } from "@/components/chrome/status-dot"
import { dateTime } from "@/lib/format"
import type { MapNode } from "@/lib/network-map"
import { freshnessLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { freshnessColor, locationSourceLabel, placeText } from "./-labels"

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
        <StatusDot
          color={freshnessColor(node.freshness)}
          shape={node.freshness === "current" ? "live" : "hollow"}
        />
        <span className="min-w-0 flex-1 truncate text-row font-medium">
          {node.name}
        </span>
        <span className="shrink-0 text-meta text-muted-foreground">
          {freshnessLabel[node.freshness]()}
        </span>
      </div>
      {location ? (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-meta">
          <dt className="text-muted-foreground">
            {m.overview_location_approximate()}
          </dt>
          <dd className="min-w-0 truncate">
            {placeText(node) ?? "—"}
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
              · {locationSourceLabel[location.endpointSource]()}
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
