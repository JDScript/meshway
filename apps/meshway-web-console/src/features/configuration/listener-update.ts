import type {
  ListenerUpdate,
  ListenerView,
  NodeSettingsView,
} from "@meshway/control-client"

// A listener's update (`setListener`) restates the whole listener: the
// controller replaces it, keeping only the secret material an update
// omits (VLESS UUID, Trojan password, a TLS certificate and key in the
// same mode) from the listener of the same key. So a form edit starts
// from the non-secret view restated as an update, secrets omitted (kept).

/**
 * The view restated as an update the controller reads as the same
 * listener; null when the view cannot restate it: WebSocket response
 * headers are shown by name only, and an update that omits them clears them.
 */
export function listenerFromView(view: ListenerView): ListenerUpdate | null {
  if (view.websocket && view.websocket.responseHeaderNames.length > 0)
    return null
  const common = {
    listenPort: view.listenPort,
    listen: view.listen,
    bindInterface: view.bindInterface,
    enabled: view.enabled,
    remark: view.remark,
    endpoints: view.endpoints.map((endpoint) => ({ ...endpoint })),
    socket: { ...view.socket },
  }
  const tcp = view.tcp ? { tcp: view.tcp } : {}
  const tls = view.tls
    ? {
        tls: {
          serverName: view.tls.serverName,
          mode: view.tls.mode,
          alpn: [...view.tls.alpn],
          minVersion: view.tls.minVersion,
          maxVersion: view.tls.maxVersion,
          handshakeTimeout: view.tls.handshakeTimeout,
        },
      }
    : null
  switch (view.protocol) {
    case "native_udp":
      return { type: "native_udp", ...common }
    case "native_tcp":
      return { type: "native_tcp", ...common, ...tcp }
    case "vless":
      if (!tls) return null
      return {
        type: "vless",
        ...common,
        ...tcp,
        ...tls,
        ...(view.flow ? { flow: view.flow } : {}),
        ...(view.stream ? { stream: view.stream } : {}),
        ...(view.websocket
          ? {
              websocket: {
                path: view.websocket.path,
                ...(view.websocket.host ? { host: view.websocket.host } : {}),
              },
            }
          : {}),
      }
    case "trojan":
      if (!tls) return null
      return {
        type: "trojan",
        ...common,
        ...tcp,
        ...tls,
        ...(view.stream === "tcp" ? { stream: "tcp" as const } : {}),
      }
  }
}

/** The listener a node's settings hold under `key`, if any. */
export const listenerViewOf = (
  settings: NodeSettingsView | undefined,
  key: string
): ListenerView | undefined =>
  settings && Object.hasOwn(settings.listeners, key)
    ? settings.listeners[key]
    : undefined

/** An update carries secret material of its own (it overwrites the kept one). */
export function writesSecrets(update: ListenerUpdate): boolean {
  if (update.type === "vless" && update.uuid !== undefined) return true
  if (update.type === "trojan" && update.password !== undefined) return true
  if (update.type === "vless" || update.type === "trojan")
    return (
      update.tls.certificate !== undefined ||
      update.tls.privateKey !== undefined
    )
  return false
}
