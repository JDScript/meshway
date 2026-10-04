import type { ListenerView } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"

import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Skeleton } from "@/components/ui/skeleton"
import { configurationQuery, isDenied } from "@/features/configuration/queries"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { protocolLabel } from "./labels"

/** `host:port`, with IPv6 in brackets. */
const endpointText = (host: string, port: number) =>
  host.includes(":") ? `[${host}]:${port}` : `${host}:${port}`

/**
 * The listeners the node's configuration holds (`nodes[id].listeners`, the
 * non-secret view), read only. What the node actually opened is State,
 * which this controller does not report.
 */
export function NodeListeners({
  networkId,
  nodeId,
}: {
  networkId: string
  nodeId: string
}) {
  const configuration = useQuery(configurationQuery(networkId))
  const settings = configuration.data?.configuration.nodes[nodeId]
  const listeners = settings
    ? Object.entries(settings.listeners).sort(([a], [b]) => a.localeCompare(b))
    : null

  return (
    <Section
      title={m.nodes_listeners()}
      count={listeners ? listeners.length : configuration.data ? 0 : null}
    >
      {configuration.data === undefined ? (
        configuration.isError ? (
          <EmptyLine>
            {isDenied(configuration.error)
              ? m.nodes_listeners_denied()
              : (failureText(configuration.error) ??
                m.nodes_listeners_failed())}
          </EmptyLine>
        ) : (
          <Skeleton className="h-12 w-full rounded-tile" />
        )
      ) : listeners === null ? (
        <EmptyLine>{m.nodes_listeners_missing()}</EmptyLine>
      ) : listeners.length === 0 ? (
        <EmptyLine>{m.nodes_listeners_none()}</EmptyLine>
      ) : (
        <div className="flex flex-col gap-6">
          {listeners.map(([key, listener]) => (
            <ListenerFacts key={key} name={key} listener={listener} />
          ))}
        </div>
      )}
    </Section>
  )
}

function ListenerFacts({
  name,
  listener,
}: {
  name: string
  listener: ListenerView
}) {
  const kind =
    listener.protocol === "vless" && listener.stream === "websocket"
      ? m.nodes_preset_vless_websocket()
      : protocolLabel[listener.protocol]()
  return (
    <div className="flex flex-col gap-2">
      <div className="flex h-6 min-w-0 items-center gap-2 text-row">
        <span className="truncate font-mono font-medium">{name}</span>
        <span className="shrink-0 text-muted-foreground">{kind}</span>
        {listener.enabled ? null : (
          <span className="shrink-0 rounded-full bg-fill-raised px-1.5 text-meta text-muted-foreground">
            {m.nodes_listener_off()}
          </span>
        )}
      </div>
      <div className="grid grid-cols-2 gap-2">
        <Tile label={m.nodes_listener_bind()}>
          <span className="font-mono font-normal">
            {endpointText(listener.listen, listener.listenPort)}
          </span>
        </Tile>
        <Tile label={m.nodes_listener_endpoints()}>
          {listener.endpoints.length === 0 ? (
            <span className="font-normal text-muted-foreground">
              {m.nodes_listener_endpoints_none()}
            </span>
          ) : (
            <span className="font-mono font-normal">
              {listener.endpoints
                .map((endpoint) => endpointText(endpoint.host, endpoint.port))
                .join(", ")}
            </span>
          )}
        </Tile>
        {listener.tls ? (
          <Tile label={m.nodes_listener_server_name()}>
            <span className="font-mono font-normal">
              {listener.tls.serverName}
            </span>
          </Tile>
        ) : null}
        {listener.websocket ? (
          <Tile label={m.nodes_listener_ws_path()}>
            <span className="font-mono font-normal">
              {listener.websocket.path}
              {listener.websocket.host ? ` · ${listener.websocket.host}` : ""}
            </span>
          </Tile>
        ) : null}
        {listener.remark ? (
          <div className="col-span-2">
            <Tile label={m.nodes_listener_remark()}>{listener.remark}</Tile>
          </div>
        ) : null}
      </div>
    </div>
  )
}
