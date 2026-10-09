import type { NodeState } from "@meshway/control-client"

import { EmptyLine, Section } from "@/components/chrome/section"
import type { GeoOf } from "@/lib/geo"
import {
  listenerRuntime,
  otherSockets,
  unconfiguredListeners,
} from "@/lib/nodes/state-model"
import { resourceText } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

import { Runtime, SocketBlock } from "./runtime"

/** What the report has that no configured listener names: dial and coordination sockets, listeners not configured. */
export function ReportedOnly({
  report,
  geoOf,
  configured,
  now,
}: {
  report: NodeState
  geoOf: GeoOf
  /** The listener keys the current configuration holds; null when not readable. */
  configured: readonly string[] | null
  now: number
}) {
  const sockets = otherSockets(report)
  const extra =
    configured === null ? [] : unconfiguredListeners(report, configured)
  return (
    <>
      {extra.length > 0 ? (
        <Section title={m.nodes_listeners_unconfigured()} count={extra.length}>
          <div className="flex flex-col gap-2">
            {extra.map((key) => {
              const runtime = listenerRuntime(report, key)
              return runtime ? (
                <div key={key} className="flex flex-col gap-1">
                  <span className="px-1 font-mono text-row">{key}</span>
                  <Runtime runtime={runtime} geoOf={geoOf} now={now} />
                </div>
              ) : null
            })}
          </div>
        </Section>
      ) : null}
      <Section title={m.nodes_sockets_other()} count={sockets.length}>
        {sockets.length === 0 ? (
          <EmptyLine>{m.nodes_sockets_other_none()}</EmptyLine>
        ) : (
          <div className="flex flex-col gap-2">
            {sockets.map(({ id, socket }) => (
              <div key={id} className="flex flex-col gap-1">
                <span className="px-1 text-meta text-muted-foreground">
                  {resourceText(socket.resource)}
                </span>
                <SocketBlock id={id} socket={socket} geoOf={geoOf} now={now} />
              </div>
            ))}
          </div>
        )}
      </Section>
    </>
  )
}
