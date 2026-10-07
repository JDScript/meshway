import type {
  Endpoint,
  ListenerUpdate,
  ListenerView,
  NodeState,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { PencilIcon, PlusIcon, Trash2Icon, Undo2Icon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { failureText } from "@/api/failure"
import { transportCatalogQuery } from "@/api/queries/nodes"
import { DraftMark } from "@/components/chrome/draft-mark"
import { IconAction } from "@/components/chrome/icon-action"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Notes, notesWithin } from "@/components/configuration/form-kit"
import { Skeleton } from "@/components/ui/skeleton"
import { pointerOf } from "@/lib/configuration/jsonc"
import { listenerFromView } from "@/lib/configuration/listener-update"
import {
  isDeleted,
  listenerEdits,
  listenerKeys,
  withListener,
  type ListenerEdit,
} from "@/lib/configuration/node-operations"
import type { GeoOf } from "@/lib/geo"
import {
  draftOfView,
  editable as formEditable,
  headerNames,
  newDraft,
  presetOfView,
  type ListenerDraft,
  type ListenerOrigin,
} from "@/lib/nodes/listener-edit"
import {
  endpointText,
  LISTENER_KEY,
  newListener,
  presetsIn,
  type Preset,
} from "@/lib/nodes/listener-model"
import { listenerRuntime } from "@/lib/nodes/state-model"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { useNodeConfiguration } from "../-draft"
import { presetLabel, protocolLabel } from "../../-labels"
import { ListenerEditor } from "./listener-editor"
import { Runtime } from "./runtime"

type Facts = {
  kind: string
  enabled: boolean
  bind: string
  endpoints: Endpoint[]
  serverName: string | null
  websocket: string | null
  /** The WebSocket block's response headers, by name. */
  headers: string[]
  remark: string
  secret: string | null
}

function factsOfView(view: ListenerView): Facts {
  const preset = presetOfView(view)
  return {
    kind: preset ? presetLabel[preset]() : protocolLabel[view.protocol](),
    enabled: view.enabled,
    bind: endpointText(view.listen, view.listenPort),
    endpoints: view.endpoints,
    serverName: view.tls?.serverName ?? null,
    websocket: view.websocket
      ? `${view.websocket.path}${view.websocket.host ? ` · ${view.websocket.host}` : ""}`
      : null,
    headers: headerNames(view),
    remark: view.remark,
    secret:
      view.protocol === "vless" || view.protocol === "trojan"
        ? [
            view.credentialPresent
              ? m.nodes_secret_credential_present()
              : m.nodes_secret_credential_absent(),
            view.tls
              ? view.tls.mode === "managed"
                ? m.nodes_tls_managed()
                : view.tls.certificatePresent
                  ? m.nodes_tls_provided()
                  : m.nodes_tls_provided_missing()
              : null,
          ]
            .filter(Boolean)
            .join(" · ")
        : null,
  }
}

/**
 * `fresh`: a listener the base does not hold (its material is generated).
 * `headers`: the configured listener's response headers, kept while it
 * stays the same listener over WebSocket.
 */
function factsOfUpdate(
  listener: ListenerUpdate,
  recreate: boolean,
  fresh: boolean,
  headers: string[]
): Facts {
  const kind =
    listener.type === "vless"
      ? presetLabel[
          listener.stream === "websocket" ? "vless_websocket" : "vless_tcp"
        ]()
      : protocolLabel[listener.type]()
  const tls =
    listener.type === "vless" || listener.type === "trojan"
      ? listener.tls
      : null
  const writes =
    (listener.type === "vless" && listener.uuid !== undefined) ||
    tls?.certificate !== undefined
  return {
    kind,
    enabled: listener.enabled ?? true,
    bind: endpointText(
      listener.listen ?? m.nodes_listener_listen_default(),
      listener.listenPort
    ),
    endpoints: listener.endpoints ?? [],
    serverName: tls?.serverName ?? null,
    websocket:
      listener.type === "vless" && listener.stream === "websocket"
        ? `${listener.websocket?.path ?? m.nodes_listener_ws_path_default()}${listener.websocket?.host ? ` · ${listener.websocket.host}` : ""}`
        : null,
    headers:
      !recreate &&
      !fresh &&
      listener.type === "vless" &&
      listener.stream === "websocket" &&
      listener.websocket !== null
        ? headers
        : [],
    remark: listener.remark ?? "",
    secret:
      listener.type === "vless" || listener.type === "trojan"
        ? writes
          ? m.nodes_secret_written()
          : recreate || fresh
            ? m.nodes_secret_regenerate()
            : m.nodes_secret_kept()
        : null,
  }
}

export function ConfiguredListeners({
  nodeId,
  configuration,
  report,
  geoOf,
  now,
}: {
  nodeId: string
  configuration: ReturnType<typeof useNodeConfiguration>
  report: NodeState | null
  geoOf: GeoOf
  now: number
}) {
  const { state, change } = configuration
  const catalog = useQuery(transportCatalogQuery())
  const presets = catalog.data
    ? presetsIn(catalog.data.items.map((item) => item.protocol))
    : []
  const [editing, setEditing] = useState<string | null>(null)

  if (state.kind === "loading")
    return (
      <Section title={m.nodes_listeners()} count={null}>
        <Skeleton className="h-12 w-full rounded-tile" />
      </Section>
    )
  if (
    state.kind === "failed" ||
    state.kind === "missing" ||
    state.kind === "unreadable"
  )
    return (
      <Section title={m.nodes_listeners()} count={null}>
        <EmptyLine>
          {state.kind === "missing"
            ? m.nodes_listeners_missing()
            : state.kind === "unreadable"
              ? m.nodes_settings_unreadable()
              : (failureText(state.error) ?? m.nodes_listeners_failed())}
        </EmptyLine>
      </Section>
    )
  if (state.kind === "member") {
    const listeners = Object.entries(state.settings.listeners).sort(
      ([a], [b]) => a.localeCompare(b)
    )
    return (
      <Section title={m.nodes_listeners()} count={listeners.length}>
        {listeners.length === 0 ? (
          <EmptyLine>{m.nodes_listeners_none()}</EmptyLine>
        ) : (
          <div className="flex flex-col gap-6">
            {listeners.map(([key, view]) => (
              <ListenerBlock
                key={key}
                name={key}
                facts={factsOfView(view)}
                mark={null}
                report={report}
                geoOf={geoOf}
                now={now}
                actions={null}
                notes={null}
              />
            ))}
          </div>
        )}
      </Section>
    )
  }

  const { base, ops, editable } = state
  const settings = base.nodes[nodeId]
  const deleted = isDeleted(ops, nodeId)
  const edits = listenerEdits(ops, nodeId)
  const keys = [
    ...new Set([...Object.keys(settings?.listeners ?? {}), ...edits.keys()]),
  ].sort()
  const live = listenerKeys(ops, base, nodeId)
  const canEdit = editable && !deleted
  const set = (key: string, edit: ListenerEdit | null) =>
    change((current, over) => withListener(current, over, nodeId, key, edit))

  const editorFor = (key: string | null) => {
    const view =
      key !== null && settings && Object.hasOwn(settings.listeners, key)
        ? settings.listeners[key]
        : undefined
    const edit = key !== null ? edits.get(key) : undefined
    const origin: ListenerOrigin = view
      ? { view, restated: listenerFromView(view) }
      : { view: null, restated: null }
    let initial: ListenerDraft | null
    if (edit?.kind === "set") {
      // Start from what the draft holds: its update, as a view would show it.
      initial =
        draftOfUpdate(edit.listener, edit.recreate) ??
        (view ? draftOfView(view) : null)
    } else
      initial = view
        ? draftOfView(view)
        : newDraft(newListener(presets[0] ?? "native_udp"))
    if (!initial) return null
    return (
      <ListenerEditor
        idPrefix={`listener-${(key ?? "new").replace(/[^A-Za-z0-9_-]/g, "_")}`}
        origin={origin}
        initial={initial}
        presets={presets.length > 0 ? presets : [initial.fields.preset]}
        takenKeys={live}
        newKey={key === null ? (preset) => suggestKey(preset, live) : null}
        onCancel={() => setEditing(null)}
        onDone={(newKey, listener, recreate) => {
          const target = key ?? newKey
          if (target === null) return
          set(target, { kind: "set", listener, recreate })
          setEditing(null)
        }}
      />
    )
  }

  return (
    <Section
      title={m.nodes_listeners()}
      count={live.length}
      action={
        canEdit && editing === null ? (
          <IconAction
            label={m.nodes_listener_add()}
            icon={PlusIcon}
            onClick={() => setEditing("")}
          />
        ) : null
      }
    >
      <div className="flex flex-col gap-6">
        {keys.length === 0 && editing !== "" ? (
          <EmptyLine>{m.nodes_listeners_none()}</EmptyLine>
        ) : null}
        {keys.map((key) => {
          const view =
            settings && Object.hasOwn(settings.listeners, key)
              ? settings.listeners[key]
              : undefined
          const edit = edits.get(key)
          if (editing === key) return <div key={key}>{editorFor(key)}</div>
          const facts =
            edit?.kind === "set"
              ? factsOfUpdate(
                  edit.listener,
                  edit.recreate,
                  !view,
                  view ? headerNames(view) : []
                )
              : view
                ? factsOfView(view)
                : null
          if (!facts) return null
          const mark =
            edit === undefined
              ? null
              : edit.kind === "remove"
                ? m.nodes_listener_mark_removed()
                : !view
                  ? m.nodes_listener_mark_added()
                  : edit.recreate
                    ? m.nodes_listener_mark_recreated()
                    : m.nodes_listener_mark_changed()
          const offered = view ? formEditable(view) : true
          return (
            <ListenerBlock
              key={key}
              name={key}
              facts={facts}
              mark={mark}
              removed={edit?.kind === "remove"}
              report={report}
              geoOf={geoOf}
              now={now}
              notes={notesWithin(
                state.notes,
                pointerOf(["nodes", nodeId, "listeners", key])
              )}
              actions={
                canEdit ? (
                  <>
                    {edit !== undefined ? (
                      <IconAction
                        label={m.nodes_listener_undo()}
                        icon={Undo2Icon}
                        onClick={() => set(key, null)}
                      />
                    ) : null}
                    {edit?.kind !== "remove" ? (
                      <>
                        <IconAction
                          label={
                            offered
                              ? m.nodes_listener_edit()
                              : m.nodes_listener_uneditable_transport()
                          }
                          icon={PencilIcon}
                          disabled={!offered}
                          onClick={() => setEditing(key)}
                        />
                        <IconAction
                          label={m.nodes_listener_remove()}
                          icon={Trash2Icon}
                          onClick={() =>
                            set(key, view ? { kind: "remove" } : null)
                          }
                        />
                      </>
                    ) : null}
                  </>
                ) : null
              }
            />
          )
        })}
        {editing === "" ? editorFor(null) : null}
      </div>
    </Section>
  )
}

/** A draft from an update the draft holds (secrets it writes are not shown again). */
function draftOfUpdate(
  listener: ListenerUpdate,
  recreate: boolean
): ListenerDraft | null {
  const view: ListenerView = {
    protocol: listener.type,
    listen: listener.listen ?? "",
    listenPort: listener.listenPort,
    bindInterface: listener.bindInterface ?? "",
    enabled: listener.enabled ?? true,
    remark: listener.remark ?? "",
    endpoints: listener.endpoints ?? [],
    socket: listener.socket ?? {},
    credentialPresent: false,
    ...(listener.type === "vless"
      ? {
          stream: listener.stream ?? "tcp",
          tls: {
            serverName: listener.tls.serverName,
            mode: listener.tls.mode ?? "managed",
            alpn: listener.tls.alpn ?? [],
            minVersion: listener.tls.minVersion ?? "1.2",
            maxVersion: listener.tls.maxVersion ?? "1.3",
            handshakeTimeout: listener.tls.handshakeTimeout ?? "10s",
            certificatePresent: false,
            privateKeyPresent: false,
          },
          websocket:
            listener.stream === "websocket"
              ? {
                  path: listener.websocket?.path ?? "",
                  host: listener.websocket?.host ?? null,
                  responseHeaderNames: [],
                }
              : null,
        }
      : {}),
  }
  const draft = draftOfView(view)
  return draft && { ...draft, recreate }
}

/** The preset's usual key, numbered when taken. */
function suggestKey(preset: Preset, taken: readonly string[]): string {
  const stem = LISTENER_KEY[preset]
  if (!taken.includes(stem)) return stem
  for (let index = 2; ; index++)
    if (!taken.includes(`${stem}-${index}`)) return `${stem}-${index}`
}

function ListenerBlock({
  name,
  facts,
  mark,
  removed = false,
  report,
  geoOf,
  now,
  actions,
  notes,
}: {
  name: string
  facts: Facts
  mark: string | null
  removed?: boolean
  report: NodeState | null
  geoOf: GeoOf
  now: number
  actions: ReactNode
  notes: ReturnType<typeof notesWithin> | null
}) {
  const runtime = report ? listenerRuntime(report, name) : null
  return (
    <div className="flex flex-col gap-2">
      <div className="flex h-8 min-w-0 items-center gap-2 text-row">
        <span
          className={cn(
            "truncate font-mono font-medium",
            removed && "text-muted-foreground line-through"
          )}
        >
          {name}
        </span>
        <span className="shrink-0 text-muted-foreground">{facts.kind}</span>
        {facts.enabled ? null : (
          <span className="shrink-0 rounded-full bg-fill-raised px-1.5 text-meta text-muted-foreground">
            {m.nodes_listener_off()}
          </span>
        )}
        {mark ? (
          <span className="inline-flex shrink-0 items-center gap-1.5 text-meta text-draft">
            <DraftMark />
            {mark}
          </span>
        ) : null}
        <span className="flex-1" />
        <span className="-mr-2 flex shrink-0 items-center gap-0.5">
          {actions}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
        <Tile label={m.nodes_listener_bind()}>
          <span className="font-mono font-normal">{facts.bind}</span>
        </Tile>
        <Tile label={m.nodes_listener_endpoints()}>
          {facts.endpoints.length === 0 ? (
            <span className="font-normal text-muted-foreground">
              {m.nodes_listener_endpoints_none()}
            </span>
          ) : (
            <span className="font-mono font-normal">
              {facts.endpoints
                .map((endpoint) => endpointText(endpoint.host, endpoint.port))
                .join(", ")}
            </span>
          )}
        </Tile>
        {facts.serverName !== null ? (
          <Tile label={m.nodes_listener_server_name()}>
            <span className="font-mono font-normal">{facts.serverName}</span>
          </Tile>
        ) : null}
        {facts.websocket !== null ? (
          <Tile label={m.nodes_listener_ws_path()}>
            <span className="font-mono font-normal">{facts.websocket}</span>
          </Tile>
        ) : null}
        {facts.headers.length > 0 ? (
          <Tile label={m.nodes_listener_ws_headers()}>
            <span className="font-mono font-normal">
              {facts.headers.join(", ")}
            </span>
          </Tile>
        ) : null}
        {facts.secret !== null ? (
          <div className="col-span-2">
            <Tile label={m.nodes_listener_material()}>
              <span className="font-normal">{facts.secret}</span>
            </Tile>
          </div>
        ) : null}
        {facts.remark ? (
          <div className="col-span-2">
            <Tile label={m.nodes_listener_remark()}>{facts.remark}</Tile>
          </div>
        ) : null}
      </div>
      {notes ? <Notes notes={notes} /> : null}
      {report ? (
        runtime ? (
          <Runtime runtime={runtime} geoOf={geoOf} now={now} />
        ) : (
          <EmptyLine size="sm">{m.nodes_listener_unreported()}</EmptyLine>
        )
      ) : null}
    </div>
  )
}
