import type { Listener, TransportProtocol } from "@meshway/control-client"
import { zListener } from "@meshway/control-client/schemas"

// A new node's one listener as it is typed, turned into the generated
// `Listener` when the enrolment key is created. Only what makes a listener
// reachable is asked: the transport, the port, an optional public address
// (and its own port behind a NAT), the bind address, and for VLESS its TLS
// name. Everything else (UUID, certificate, WebSocket path and Host,
// socket and TCP options) is left out, so the controller fills its
// defaults and generates the material. The generated schema decides what
// it can; the rules it cannot express (an IP to bind, a host name) follow
// docs/v2/configuration-reference/shared/listen.md and the controller
// still decides, when the node enrols.

export const PRESETS = [
  "native_udp",
  "native_tcp",
  "vless_tcp",
  "vless_websocket",
] as const
export type Preset = (typeof PRESETS)[number]

export const protocolOf = (preset: Preset): TransportProtocol =>
  preset === "vless_tcp" || preset === "vless_websocket" ? "vless" : preset

/** The presets whose protocol the controller's transport catalog offers. */
export const presetsIn = (protocols: readonly TransportProtocol[]): Preset[] =>
  PRESETS.filter((preset) => protocols.includes(protocolOf(preset)))

/** The listener's key in `nodes[id].listeners`. */
export const LISTENER_KEY: Record<Preset, string> = {
  native_udp: "udp",
  native_tcp: "tcp",
  vless_tcp: "vless",
  vless_websocket: "vless-ws",
}

/** The port a preset suggests: 443 where TLS is spoken, else Meshway's own. */
export const suggestedPort = (preset: Preset) =>
  protocolOf(preset) === "vless" ? "443" : "8421"

export type ListenerFields = {
  preset: Preset
  /** The bind port. */
  port: string
  /** The public address dialers use; empty: none is announced. */
  host: string
  /** The public address's own port; empty: the bind port. */
  publicPort: string
  /** The bind address; empty: the controller's default (all IPv4). */
  listen: string
  /** VLESS: the TLS name; empty: the public address. */
  serverName: string
}

export const newListener = (preset: Preset): ListenerFields => ({
  preset,
  port: suggestedPort(preset),
  host: "",
  publicPort: "",
  listen: "",
  serverName: "",
})

/** Another transport; a port still at the old suggestion follows the new one. */
export function withPreset(
  fields: ListenerFields,
  preset: Preset
): ListenerFields {
  const port =
    fields.port.trim() === "" ||
    fields.port.trim() === suggestedPort(fields.preset)
      ? suggestedPort(preset)
      : fields.port
  return { ...fields, preset, port }
}

export type ListenerProblem =
  | "port"
  | "host"
  | "public_port"
  | "listen"
  | "server_name"
  | "server_name_missing"

export type ListenerField = Exclude<keyof ListenerFields, "preset">

export const problemField: Record<ListenerProblem, ListenerField> = {
  port: "port",
  host: "host",
  public_port: "publicPort",
  listen: "listen",
  server_name: "serverName",
  server_name_missing: "serverName",
}

const PORT = /^[0-9]{1,5}$/
const IPV4 =
  /^(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])(\.(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])){3}$/
const LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/

function portOf(raw: string): number | null {
  const text = raw.trim()
  if (!PORT.test(text)) return null
  const port = Number(text)
  return port >= 1 && port <= 65535 ? port : null
}

function isIpv6(value: string): boolean {
  if (!value.includes(":") || !/^[0-9A-Fa-f:.]+$/.test(value)) return false
  try {
    return new URL(`http://[${value}]/`).hostname !== ""
  } catch {
    return false
  }
}

export const isIp = (value: string) => IPV4.test(value) || isIpv6(value)

/**
 * A host as the controller reads one: an IP, or a DNS name of letters,
 * digits and inner hyphens per label (an international name in its ASCII
 * form), at most 253 characters, one trailing dot allowed.
 */
export function isHost(raw: string): boolean {
  const value = raw.trim()
  if (value === "") return false
  if (isIp(value)) return true
  // No scheme, port, path, user or escapes: only the name itself.
  if (/[/:?#@[\]\\%\s]/.test(value)) return false
  let ascii: string
  try {
    ascii = new URL(`http://${value.replace(/\.$/, "")}/`).hostname
  } catch {
    return false
  }
  return (
    ascii.length > 0 &&
    ascii.length <= 253 &&
    ascii.split(".").every((label) => LABEL.test(label))
  )
}

/** What stops creating the listener, in form order. */
export function listenerProblems(fields: ListenerFields): ListenerProblem[] {
  const problems: ListenerProblem[] = []
  if (portOf(fields.port) === null) problems.push("port")
  const host = fields.host.trim()
  if (host !== "" && !isHost(host)) problems.push("host")
  if (
    host !== "" &&
    fields.publicPort.trim() !== "" &&
    portOf(fields.publicPort) === null
  )
    problems.push("public_port")
  const listen = fields.listen.trim()
  if (listen !== "" && !isIp(listen)) problems.push("listen")
  if (protocolOf(fields.preset) === "vless") {
    const name = fields.serverName.trim()
    if (name !== "" && !isHost(name)) problems.push("server_name")
    else if (name === "" && host === "") problems.push("server_name_missing")
  }
  return problems
}

/** The fields under "advanced" that differ from what the controller would use. */
export const advancedSet = (fields: ListenerFields) =>
  fields.listen.trim() !== "" ||
  fields.publicPort.trim() !== "" ||
  fields.serverName.trim() !== ""

/**
 * The generated `Listener` for valid fields (null otherwise): only what was
 * chosen or typed, nothing the controller fills or generates.
 */
export function listenerOf(fields: ListenerFields): Listener | null {
  if (listenerProblems(fields).length > 0) return null
  const listenPort = portOf(fields.port)
  if (listenPort === null) return null
  const host = fields.host.trim()
  const common = {
    listenPort,
    ...(fields.listen.trim() === "" ? {} : { listen: fields.listen.trim() }),
    ...(host === ""
      ? {}
      : {
          endpoints: [{ host, port: portOf(fields.publicPort) ?? listenPort }],
        }),
  }
  const candidate =
    fields.preset === "native_udp" || fields.preset === "native_tcp"
      ? { type: fields.preset, ...common }
      : {
          type: "vless" as const,
          ...common,
          ...(fields.preset === "vless_websocket"
            ? { stream: "websocket" as const }
            : {}),
          tls: { serverName: fields.serverName.trim() || host },
        }
  const parsed = zListener.safeParse(candidate)
  return parsed.success ? parsed.data : null
}
