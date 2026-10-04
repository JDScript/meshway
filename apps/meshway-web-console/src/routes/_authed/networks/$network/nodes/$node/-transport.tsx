import type {
  Endpoint,
  EndpointCandidate,
  Listener,
  ListenerView,
  NetworkSummary,
  NodeInventoryResponse,
  NodeState,
  SocketState,
  StateView,
} from "@meshway/control-client"
import { useQuery, type UseQueryResult } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { PencilIcon, PlusIcon, Trash2Icon, Undo2Icon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { failureText } from "@/api/failure"
import { transportCatalogQuery } from "@/api/queries/nodes"
import { DraftMark } from "@/components/chrome/draft-mark"
import { IconAction } from "@/components/chrome/icon-action"
import { RefList, RefRow } from "@/components/chrome/ref-row"
import { EmptyLine, Section } from "@/components/chrome/section"
import { StatusDot } from "@/components/chrome/status-dot"
import { Tile } from "@/components/chrome/tile"
import { Notes, notesWithin } from "@/components/configuration/form-kit"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { pointerOf } from "@/lib/configuration/jsonc"
import { listenerFromView } from "@/lib/configuration/listener-update"
import {
  isDeleted,
  listenerEdits,
  listenerKeys,
  withListener,
  type ListenerEdit,
} from "@/lib/configuration/node-operations"
import { dateTime, relativeTime } from "@/lib/format"
import {
  draftOfView,
  newDraft,
  presetOfView,
  uneditable,
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
import {
  candidateExpired,
  isPast,
  listenerRuntime,
  nodeLinks,
  nodePaths,
  otherSockets,
  reportOf,
  reportTime,
  stateOf,
  unconfiguredListeners,
} from "@/lib/nodes/state-model"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { presetLabel, protocolLabel } from "../-labels"
import {
  endpointSourceText,
  evidenceLabel,
  freshnessLabel,
  linkPhaseLabel,
  listenerPhaseLabel,
  pathEvidenceLabel,
  problemText,
  reporterRoleLabel,
  resourceText,
} from "../-state-labels"
import { useNodeConfiguration } from "./-draft"
import { ListenerEditor } from "./-listener-editor"

/** A time as when, with how long ago or until; "expired" once past. */
function TimeFact({
  at,
  now,
  expiry = false,
}: {
  at: string | null | undefined
  now: number
  expiry?: boolean
}) {
  if (!at) return <span className="text-muted-foreground">—</span>
  const past = isPast(at, now)
  return (
    <span className={cn(expiry && past && "text-warning")} title={at}>
      {expiry && past
        ? m.nodes_time_expired({ at: dateTime(at) ?? at })
        : (relativeTime(at, now) ?? at)}
    </span>
  )
}

/**
 * A node's transport: the listeners its configuration holds (edited here
 * into the draft) with what the node reports about each, the other sockets
 * it reports, and its links and paths as the controller aggregates them.
 */
export function NodeTransport({
  networkId,
  nodeId,
  network,
  state,
  nodes,
}: {
  networkId: string
  nodeId: string
  network: NetworkSummary | null
  state: UseQueryResult<StateView>
  nodes: NodeInventoryResponse | undefined
}) {
  const now = useNow(10_000)
  const configuration = useNodeConfiguration(networkId, nodeId, network)
  const view = stateOf(state.data, nodeId)
  const report = reportOf(view)
  const names = (id: string) => {
    const configured =
      configuration.state.kind === "manager"
        ? configuration.state.base.nodes[id]?.hostname
        : undefined
    return (
      configured ??
      nodes?.items.find((node) => node.id === id)?.hostname ??
      id.slice(0, 8)
    )
  }
  const full = report.view === "full" ? report.report : null

  return (
    <div className="flex flex-col gap-8">
      <ReportLine
        loading={state.data === undefined && !state.isError}
        failed={
          state.isError
            ? (failureText(state.error) ?? m.nodes_state_failed())
            : null
        }
        view={view}
        reportView={report.view}
        now={now}
      />
      <ConfiguredListeners
        nodeId={nodeId}
        configuration={configuration}
        report={full}
        now={now}
      />
      {full ? (
        <ReportedOnly
          report={full}
          configured={
            configuration.state.kind === "manager"
              ? Object.keys(
                  configuration.state.view.configuration.nodes[nodeId]
                    ?.listeners ?? {}
                )
              : configuration.state.kind === "member"
                ? Object.keys(configuration.state.settings.listeners)
                : null
          }
          now={now}
        />
      ) : null}
      {report.view === "peer" ? (
        <Section
          title={m.nodes_peer_endpoints()}
          count={report.report.endpoints.length}
        >
          {report.report.endpoints.length === 0 ? (
            <EmptyLine>{m.nodes_endpoints_none()}</EmptyLine>
          ) : (
            <ul className="flex flex-col gap-1">
              {report.report.endpoints.map((endpoint) => (
                <li
                  key={`${endpoint.socketId}-${endpoint.candidate.address}`}
                  className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 rounded-tile bg-fill-subtle px-3.5 py-2 text-row"
                >
                  <span className="truncate font-mono">
                    {endpoint.candidate.address}
                  </span>
                  <span className="text-meta text-muted-foreground">
                    {endpoint.network.toUpperCase()} ·{" "}
                    {resourceText(endpoint.resource)}
                  </span>
                  <span className="col-span-2 text-meta text-muted-foreground">
                    <TimeFact
                      at={endpoint.candidate.expiresAt}
                      now={now}
                      expiry
                    />
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Section>
      ) : null}
      <Links
        networkId={networkId}
        nodeId={nodeId}
        state={state.data}
        names={names}
        nodes={nodes}
        now={now}
      />
      <Paths nodeId={nodeId} state={state.data} names={names} now={now} />
    </div>
  )
}

function ReportLine({
  loading,
  failed,
  view,
  reportView,
  now,
}: {
  loading: boolean
  failed: string | null
  view: ReturnType<typeof stateOf>
  reportView: "full" | "peer" | "none"
  now: number
}) {
  if (loading) return <Skeleton className="h-12 w-full rounded-tile" />
  if (failed !== null) return <EmptyLine>{failed}</EmptyLine>
  if (!view) return <EmptyLine>{m.nodes_state_absent()}</EmptyLine>
  const at = reportTime(view)
  return (
    <div className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-1 rounded-tile bg-fill-subtle px-3.5 py-2 text-row">
      <StatusDot
        color={
          view.freshness === "current"
            ? "var(--vitality-live)"
            : "var(--vitality-quiet)"
        }
        shape={view.freshness === "current" ? "live" : "hollow"}
      />
      <span className="font-medium">{freshnessLabel[view.freshness]()}</span>
      <span className="text-muted-foreground">
        {at
          ? m.nodes_report_at({ at: relativeTime(at, now) ?? at })
          : m.nodes_report_none()}
      </span>
      <span className="ml-auto text-meta text-muted-foreground">
        {reportView === "full"
          ? m.nodes_report_full()
          : reportView === "peer"
            ? m.nodes_report_peer()
            : null}
      </span>
    </div>
  )
}

type Facts = {
  kind: string
  enabled: boolean
  bind: string
  endpoints: Endpoint[]
  serverName: string | null
  websocket: string | null
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

/** `fresh`: a listener the base does not hold (its material is generated). */
function factsOfUpdate(
  listener: Listener,
  recreate: boolean,
  fresh: boolean
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

function ConfiguredListeners({
  nodeId,
  configuration,
  report,
  now,
}: {
  nodeId: string
  configuration: ReturnType<typeof useNodeConfiguration>
  report: NodeState | null
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
              ? factsOfUpdate(edit.listener, edit.recreate, !view)
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
          const why = view ? uneditable(view) : null
          return (
            <ListenerBlock
              key={key}
              name={key}
              facts={facts}
              mark={mark}
              removed={edit?.kind === "remove"}
              report={report}
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
                            why === null
                              ? m.nodes_listener_edit()
                              : why === "transport"
                                ? m.nodes_listener_uneditable_transport()
                                : m.nodes_listener_uneditable_headers()
                          }
                          icon={PencilIcon}
                          disabled={why !== null}
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
  listener: Listener,
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
  now,
  actions,
  notes,
}: {
  name: string
  facts: Facts
  mark: string | null
  removed?: boolean
  report: NodeState | null
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
          <Runtime runtime={runtime} now={now} />
        ) : (
          <EmptyLine size="sm">{m.nodes_listener_unreported()}</EmptyLine>
        )
      ) : null}
    </div>
  )
}

function Runtime({
  runtime,
  now,
}: {
  runtime: NonNullable<ReturnType<typeof listenerRuntime>>
  now: number
}) {
  const { state } = runtime
  return (
    <div className="flex flex-col gap-2 rounded-surface border border-divider p-2">
      <div className="flex min-h-8 flex-wrap items-center gap-x-3 gap-y-1 px-1.5 text-row">
        <span className="text-meta text-muted-foreground">
          {m.nodes_runtime()}
        </span>
        <span
          className={cn(
            "font-medium",
            state.phase === "failed" && "text-destructive"
          )}
        >
          {listenerPhaseLabel[state.phase]()}
        </span>
        {state.configuration ? (
          <span className="font-mono text-meta text-muted-foreground">
            {m.revision({ revision: state.configuration.revision })}
          </span>
        ) : null}
        {state.loadedCertificateSha256 ? (
          <span
            className="font-mono text-meta text-muted-foreground"
            title={state.loadedCertificateSha256}
          >
            {m.nodes_certificate_loaded({
              hash: state.loadedCertificateSha256.slice(0, 16),
            })}
          </span>
        ) : null}
      </div>
      {state.problem ? (
        <p className="px-1.5 text-row text-destructive">
          {problemText(state.problem)}
        </p>
      ) : null}
      {runtime.sockets.length === 0 ? (
        <p className="px-1.5 text-meta text-muted-foreground">
          {m.nodes_sockets_none()}
        </p>
      ) : (
        runtime.sockets.map(({ id, socket }) =>
          socket ? (
            <SocketBlock key={id} id={id} socket={socket} now={now} />
          ) : (
            <p key={id} className="px-1.5 text-meta text-muted-foreground">
              {m.nodes_socket_missing({ id })}
            </p>
          )
        )
      )}
    </div>
  )
}

function SocketBlock({
  id,
  socket,
  now,
}: {
  id: string
  socket: SocketState
  now: number
}) {
  return (
    <div className="flex flex-col gap-1 rounded-tile bg-fill-subtle p-2">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-0.5 px-1.5 text-row">
        <span className="font-mono">{socket.boundAddress}</span>
        <span className="text-meta text-muted-foreground">
          {socket.network.toUpperCase()}
        </span>
        <span className="text-meta text-muted-foreground">
          {m.nodes_socket_generation({ generation: socket.generation })}
        </span>
        <span
          className="min-w-0 truncate font-mono text-meta text-muted-foreground"
          title={id}
        >
          {id}
        </span>
      </div>
      {socket.endpoints.length === 0 ? (
        <p className="px-1.5 text-meta text-muted-foreground">
          {m.nodes_endpoints_none()}
        </p>
      ) : (
        <ul className="flex flex-col">
          {socket.endpoints.map((candidate, index) => (
            <CandidateRow
              key={`${candidate.address}-${index}`}
              candidate={candidate}
              now={now}
            />
          ))}
        </ul>
      )}
    </div>
  )
}

function CandidateRow({
  candidate,
  now,
}: {
  candidate: EndpointCandidate
  now: number
}) {
  const expired = candidateExpired(candidate, now)
  return (
    <li
      className={cn(
        "grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-3 rounded-[10px] px-1.5 py-1 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)_auto]",
        expired && "text-muted-foreground"
      )}
    >
      <span
        className={cn("truncate font-mono text-row", expired && "line-through")}
      >
        {candidate.address}
      </span>
      <span className="truncate text-meta text-muted-foreground max-sm:col-start-1">
        {endpointSourceText(candidate.origin)}
      </span>
      <span className="text-meta text-muted-foreground tabular-nums max-sm:col-start-2 max-sm:row-start-1 sm:text-right">
        {expired ? (
          <span className="text-warning">{m.nodes_candidate_expired()}</span>
        ) : (
          <>
            {m.nodes_candidate_observed({
              at:
                relativeTime(candidate.observedAt, now) ?? candidate.observedAt,
            })}
            {" · "}
            {m.nodes_candidate_expires({
              at: relativeTime(candidate.expiresAt, now) ?? candidate.expiresAt,
            })}
          </>
        )}
      </span>
    </li>
  )
}

/** What the report has that no configured listener names: dial and coordination sockets, listeners not configured. */
function ReportedOnly({
  report,
  configured,
  now,
}: {
  report: NodeState
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
                  <Runtime runtime={runtime} now={now} />
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
                <SocketBlock id={id} socket={socket} now={now} />
              </div>
            ))}
          </div>
        )}
      </Section>
    </>
  )
}

function Links({
  networkId,
  nodeId,
  state,
  names,
  nodes,
  now,
}: {
  networkId: string
  nodeId: string
  state: StateView | undefined
  names: (id: string) => string
  nodes: NodeInventoryResponse | undefined
  now: number
}) {
  const navigate = useNavigate()
  const links = nodeLinks(state, nodeId)
  return (
    <Section title={m.nodes_links()} count={state ? links.length : null}>
      {state === undefined ? (
        <Skeleton className="h-12 w-full rounded-tile" />
      ) : links.length === 0 ? (
        <EmptyLine>{m.nodes_links_none()}</EmptyLine>
      ) : (
        <RefList>
          {links.map((link) => {
            const listed =
              nodes?.items.some((node) => node.id === link.peerId) ?? false
            return (
              <RefRow
                key={link.linkId}
                lead={
                  <StatusDot
                    color={
                      link.phase === "authenticated" &&
                      link.evidence === "matched"
                        ? "var(--vitality-live)"
                        : "var(--vitality-quiet)"
                    }
                    shape={
                      link.phase === "authenticated" &&
                      link.evidence === "matched"
                        ? "live"
                        : "hollow"
                    }
                  />
                }
                title={names(link.peerId)}
                subtitle={[
                  evidenceLabel[link.evidence](),
                  link.transport,
                  link.initiatorNodeId === nodeId
                    ? m.nodes_link_initiated_here()
                    : m.nodes_link_initiated_peer(),
                  link.inUse === true
                    ? m.nodes_link_in_use()
                    : link.inUse === false
                      ? m.nodes_link_idle()
                      : m.nodes_link_use_unknown(),
                  link.since
                    ? m.nodes_link_since({
                        at: relativeTime(link.since, now) ?? link.since,
                      })
                    : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
                state={linkPhaseLabel[link.phase]()}
                onOpen={
                  listed
                    ? () =>
                        void navigate({
                          to: "/networks/$network/nodes/$node",
                          params: { network: networkId, node: link.peerId },
                          search: { tab: "transport" },
                        })
                    : undefined
                }
              />
            )
          })}
        </RefList>
      )}
    </Section>
  )
}

function Paths({
  nodeId,
  state,
  names,
  now,
}: {
  nodeId: string
  state: StateView | undefined
  names: (id: string) => string
  now: number
}) {
  const paths = nodePaths(state, nodeId)
  if (state === undefined) return null
  return (
    <Section title={m.nodes_paths()} count={paths.length}>
      {paths.length === 0 ? (
        <EmptyLine>{m.nodes_paths_none()}</EmptyLine>
      ) : (
        <ul className="flex flex-col gap-1">
          {paths.map((path) => (
            <li
              key={`${path.pathId}-${path.reporter.nodeId}`}
              className="flex flex-col gap-0.5 rounded-tile bg-fill-subtle px-3.5 py-2"
            >
              <span className="truncate text-row font-medium">
                {path.state.nodes.map(names).join(" → ")}
              </span>
              <span className="text-meta text-muted-foreground">
                {[
                  pathEvidenceLabel[path.state.evidence](),
                  m.nodes_path_reported_by({
                    name: names(path.reporter.nodeId),
                    role: reporterRoleLabel[path.state.reporterRole](),
                  }),
                ].join(" · ")}
                {" · "}
                <TimeFact at={path.state.expiresAt} now={now} expiry />
              </span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  )
}
