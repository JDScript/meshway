import type {
  Endpoint,
  Listener,
  ListenerView,
  TlsMode,
} from "@meshway/control-client"
import { zListener } from "@meshway/control-client/schemas"

import { listenerFromView } from "../configuration/listener-update"
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
// the view (listener-update.ts), so the update keeps them.
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
  restated: Listener | null
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

/**
 * Why a configured listener cannot be edited in the form: a transport the
 * form does not offer (Trojan), or WebSocket response headers the view
 * shows by name only (restating it would clear them).
 */
export type Uneditable = "transport" | "response_headers"

export function uneditable(view: ListenerView): Uneditable | null {
  if (presetOfView(view) === null) return "transport"
  if (listenerFromView(view) === null) return "response_headers"
  return null
}

export function draftOfView(view: ListenerView): ListenerDraft | null {
  const preset = presetOfView(view)
  if (preset === null || uneditable(view) !== null) return null
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
 * restated fields, with what the form shows over them.
 */
export function updateOf(
  draft: ListenerDraft,
  origin: ListenerOrigin
): Listener | null {
  if (draftProblems(draft, origin).length > 0) return null
  const { fields } = draft
  const protocol = protocolOf(fields.preset)
  const listenPort = Number(fields.port.trim())
  const host = fields.host.trim()
  const publicPort =
    fields.publicPort.trim() === "" ? listenPort : Number(fields.publicPort)
  // Fields the form does not show come from the same transport only.
  const kept =
    origin.restated && origin.restated.type === protocol
      ? origin.restated
      : null
  const extra: Endpoint[] = kept?.endpoints?.slice(1) ?? []
  const common = {
    ...(kept ?? {}),
    listenPort,
    enabled: draft.enabled,
    remark: draft.remark,
    endpoints: host === "" ? extra : [{ host, port: publicPort }, ...extra],
  }
  const listen = fields.listen.trim()
  if (listen === "") delete (common as { listen?: string }).listen
  else (common as { listen?: string }).listen = listen

  let candidate: unknown
  if (fields.preset === "native_udp" || fields.preset === "native_tcp")
    candidate = { ...common, type: fields.preset }
  else {
    const keptTls = kept && "tls" in kept ? kept.tls : undefined
    const certificate = draft.certificate.trim()
    const tls = {
      ...(keptTls ?? {}),
      serverName: fields.serverName.trim() || host,
      mode: draft.tlsMode,
      ...(draft.tlsMode === "provided" && certificate !== ""
        ? { certificate: draft.certificate, privateKey: draft.privateKey }
        : {}),
    }
    const websocket =
      fields.preset === "vless_websocket"
        ? {
            websocket: {
              ...(draft.wsPath.trim() === ""
                ? {}
                : { path: draft.wsPath.trim() }),
              ...(draft.wsHost.trim() === ""
                ? {}
                : { host: draft.wsHost.trim() }),
            },
          }
        : {}
    const { websocket: _keptWebsocket, ...rest } = common as typeof common & {
      websocket?: unknown
    }
    void _keptWebsocket
    candidate = {
      ...rest,
      type: "vless",
      stream: fields.preset === "vless_websocket" ? "websocket" : "tcp",
      tls,
      ...websocket,
      ...(draft.uuid.trim() === "" ? {} : { uuid: draft.uuid.trim() }),
    }
  }
  const parsed = zListener.safeParse(candidate)
  return parsed.success ? parsed.data : null
}

/** The update the controller refuses by its schema, though the form found nothing. */
export const schemaRefuses = (draft: ListenerDraft, origin: ListenerOrigin) =>
  draftProblems(draft, origin).length === 0 && updateOf(draft, origin) === null

/** A listener key as the controller takes one: not empty (no edge spaces, kept as typed). */
export const isListenerKey = (key: string) => key !== "" && key === key.trim()
