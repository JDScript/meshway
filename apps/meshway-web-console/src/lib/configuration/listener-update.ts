import type {
  ListenerSecret,
  ListenerUpdate,
  ListenerView,
} from "@meshway/control-client"

// A listener's update (`setListener`) restates the whole listener: the
// controller replaces it, keeping from the listener of the same key only
// what an update omits of its secret material (VLESS UUID, Trojan
// password, a TLS certificate and key in the same mode) and of a VLESS
// WebSocket block (its response headers, which the view names only). So a
// form edit starts from the non-secret view restated as an update, those
// omitted (kept). A listener set anew (recreated) keeps none of them.

/** The view restated as an update the controller reads as the same listener; null without the TLS a VLESS or Trojan view must have. */
export function listenerFromView(view: ListenerView): ListenerUpdate | null {
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

/** A secret value of its own: `{retained: true}` keeps the saved one. */
const written = (secret: ListenerSecret | undefined) =>
  typeof secret === "string"

/** An update carries secret material of its own (it overwrites the kept one). */
export function writesSecrets(update: ListenerUpdate): boolean {
  if (update.type === "vless" && written(update.uuid)) return true
  if (update.type === "trojan" && written(update.password)) return true
  if (update.type === "vless" || update.type === "trojan")
    return (
      update.tls.certificate !== undefined || written(update.tls.privateKey)
    )
  return false
}
