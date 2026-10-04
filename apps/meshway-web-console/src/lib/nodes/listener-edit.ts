import type {
  Endpoint,
  ListenerUpdate,
  ListenerView,
  SocketOptions,
  TlsMode,
} from "@meshway/control-client"
import { zListenerUpdate } from "@meshway/control-client/schemas"

import { listenerFromView } from "@/lib/configuration/listener-update"

import {
  listenerProblems,
  protocolOf,
  type ListenerFields,
  type ListenerProblem,
  type Preset,
} from "./listener-model"

// A configured listener edited in a node's workspace, or one added to it:
// the add-node inputs (transport, port, public address, bind address, TLS
// name) plus what an existing listener also shows (on or off, remark,
// WebSocket path and Host, where its certificate comes from), turned into
// a `setListener` update. Fields the form does not show are restated from
// the view (listener-update.ts), so the update keeps them; WebSocket
// response headers are left out, so the controller keeps them.
//
// Secrets are write-only (D5): the VLESS UUID and a provided certificate
// and key are never read; empty keeps what the listener holds, a value
// overwrites it. Recreating the listener (rotate) makes the controller
// generate new material for what is not written: the UUID and a managed
// certificate. A new listener, or one whose transport changes, has no
// material to keep.

export type ListenerDraft = {
  fields: ListenerFields
  enabled: boolean
  remark: string
  /** VLESS · WS: the path; "" takes the controller's default. */
  wsPath: string
  /** VLESS · WS: the Host header; "" for none. */
  wsHost: string
  /** VLESS: the controller makes the certificate (`managed`) or it is provided. */
  tlsMode: TlsMode
  /** "" keeps (or, new or recreated, generates). */
  uuid: string
  /** PEM; "" keeps. Both or neither. */
  certificate: string
  privateKey: string
  /** Remove and set again in one apply: new generated material. */
  recreate: boolean
}

/** What the draft is edited over: the configured listener, or none for a new one. */
export type ListenerOrigin = {
  /** The view restated as an update; null for a new listener. */
  restated: ListenerUpdate | null
  /** The configured view; null for a new listener. */
  view: ListenerView | null
}

export const presetOfView = (view: ListenerView): Preset | null => {
  switch (view.protocol) {
    case "native_udp":
    case "native_tcp":
      return view.protocol
    case "vless":
      return view.stream === "websocket" ? "vless_websocket" : "vless_tcp"
    case "trojan":
      return null
  }
}

/** A configured listener the form edits: a transport it offers (not Trojan), restated as an update. */
export const editable = (view: ListenerView) =>
  presetOfView(view) !== null && listenerFromView(view) !== null

/** The response headers a VLESS listener's WebSocket block has, by name. */
export const headerNames = (view: ListenerView): string[] =>
  view.websocket?.responseHeaderNames ?? []

export function draftOfView(view: ListenerView): ListenerDraft | null {
  const preset = presetOfView(view)
  if (preset === null || !editable(view)) return null
  const first = view.endpoints[0]
  return {
    fields: {
      preset,
      port: `${view.listenPort}`,
      host: first?.host ?? "",
      publicPort:
        first && first.port !== view.listenPort ? `${first.port}` : "",
      listen: view.listen,
      serverName: view.tls?.serverName ?? "",
    },
    enabled: view.enabled,
    remark: view.remark,
    wsPath: view.websocket?.path ?? "",
    wsHost: view.websocket?.host ?? "",
    tlsMode: view.tls?.mode ?? "managed",
    uuid: "",
    certificate: "",
    privateKey: "",
    recreate: false,
  }
}

export const newDraft = (fields: ListenerFields): ListenerDraft => ({
  fields,
  enabled: true,
  remark: "",
  wsPath: "",
  wsHost: "",
  tlsMode: "managed",
  uuid: "",
  certificate: "",
  privateKey: "",
  recreate: false,
})

export type DraftProblem =
  | ListenerProblem
  | "ws_path"
  | "uuid"
  | "certificate_pair"
  | "certificate_missing"
  | "schema"

const UUID =
  /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/

/** The origin's material is kept: same transport, not recreated. */
const keeps = (draft: ListenerDraft, origin: ListenerOrigin) =>
  origin.view !== null &&
  !draft.recreate &&
  origin.view.protocol === protocolOf(draft.fields.preset)

/** A provided certificate and key the listener holds and keeps. */
export const keepsCertificate = (
  draft: ListenerDraft,
  origin: ListenerOrigin
) =>
  keeps(draft, origin) &&
  origin.view?.tls?.mode === "provided" &&
  origin.view.tls.certificatePresent &&
  origin.view.tls.privateKeyPresent

/** The listener's UUID is kept (vs generated) when none is written. */
export const keepsUuid = (draft: ListenerDraft, origin: ListenerOrigin) =>
  keeps(draft, origin) && (origin.view?.credentialPresent ?? false)

/** The fields that stop the edit, in form order. */
export function draftProblems(
  draft: ListenerDraft,
  origin: ListenerOrigin
): DraftProblem[] {
  const problems: DraftProblem[] = [...listenerProblems(draft.fields)]
  if (protocolOf(draft.fields.preset) !== "vless") return problems
  if (
    draft.fields.preset === "vless_websocket" &&
    draft.wsPath.trim() !== "" &&
    !/^\/[^\s?#]*$/.test(draft.wsPath.trim())
  )
    problems.push("ws_path")
  if (draft.uuid.trim() !== "" && !UUID.test(draft.uuid.trim()))
    problems.push("uuid")
  if (draft.tlsMode === "provided") {
    const cert = draft.certificate.trim() !== ""
    const key = draft.privateKey.trim() !== ""
    if (cert !== key) problems.push("certificate_pair")
    else if (!cert && !keepsCertificate(draft, origin))
      problems.push("certificate_missing")
  }
  return problems
}

/**
 * The update for a draft without problems (null otherwise): the origin's
 * restated fields of the same transport, with what the form shows over them.
 */
export function updateOf(
  draft: ListenerDraft,
  origin: ListenerOrigin
): ListenerUpdate | null {
  if (draftProblems(draft, origin).length > 0) return null
  const { fields } = draft
  const listenPort = Number(fields.port.trim())
  const host = fields.host.trim()
  const publicPort =
    fields.publicPort.trim() === "" ? listenPort : Number(fields.publicPort)
  const listen = fields.listen.trim()
  const before = origin.restated
  // What the form shows of every transport, over the fields it does not.
  const placed = (
    kept: {
      endpoints?: Endpoint[]
      bindInterface?: string
      socket?: SocketOptions
    } | null
  ) => ({
    listenPort,
    ...(listen === "" ? {} : { listen }),
    ...(kept?.bindInterface === undefined
      ? {}
      : { bindInterface: kept.bindInterface }),
    ...(kept?.socket === undefined ? {} : { socket: kept.socket }),
    enabled: draft.enabled,
    remark: draft.remark,
    endpoints: [
      ...(host === "" ? [] : [{ host, port: publicPort }]),
      ...(kept?.endpoints?.slice(1) ?? []),
    ],
  })

  let candidate: ListenerUpdate
  switch (fields.preset) {
    case "native_udp": {
      const kept = before?.type === "native_udp" ? before : null
      candidate = { type: "native_udp", ...placed(kept) }
      break
    }
    case "native_tcp": {
      const kept = before?.type === "native_tcp" ? before : null
      candidate = {
        type: "native_tcp",
        ...placed(kept),
        ...(kept?.tcp === undefined ? {} : { tcp: kept.tcp }),
      }
      break
    }
    case "vless_tcp":
    case "vless_websocket": {
      const kept = before?.type === "vless" ? before : null
      const websocket = fields.preset === "vless_websocket"
      const certificate = draft.certificate.trim()
      const wsPath = draft.wsPath.trim()
      const wsHost = draft.wsHost.trim()
      candidate = {
        type: "vless",
        ...placed(kept),
        ...(kept?.tcp === undefined ? {} : { tcp: kept.tcp }),
        ...(kept?.flow ? { flow: kept.flow } : {}),
        stream: websocket ? "websocket" : "tcp",
        tls: {
          ...(kept?.tls ?? {}),
          serverName: fields.serverName.trim() || host,
          mode: draft.tlsMode,
          ...(draft.tlsMode === "provided" && certificate !== ""
            ? { certificate: draft.certificate, privateKey: draft.privateKey }
            : {}),
        },
        // An omitted block (or its omitted headers) is kept: over TCP, the
        // block the listener had is removed.
        ...(websocket
          ? {
              websocket: {
                ...(wsPath === "" ? {} : { path: wsPath }),
                ...(wsHost === "" ? {} : { host: wsHost }),
              },
            }
          : kept?.websocket
            ? { websocket: null }
            : {}),
        ...(draft.uuid.trim() === "" ? {} : { uuid: draft.uuid.trim() }),
      }
      break
    }
  }
  const parsed = zListenerUpdate.safeParse(candidate)
  return parsed.success ? parsed.data : null
}

/** The update the controller refuses by its schema, though the form found nothing. */
export const schemaRefuses = (draft: ListenerDraft, origin: ListenerOrigin) =>
  draftProblems(draft, origin).length === 0 && updateOf(draft, origin) === null

/** A listener key as the controller takes one: not empty (no edge spaces, kept as typed). */
export const isListenerKey = (key: string) => key !== "" && key === key.trim()
