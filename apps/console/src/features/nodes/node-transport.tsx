import {
  saveManagedListener,
  type ConnectionsSnapshot,
  type ManagedListenersSnapshot,
  type ManagedListenerSummary,
  type NodeSummary,
} from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { ChevronDownIcon, PlusIcon } from "lucide-react"
import { useEffect, useMemo, useRef } from "react"

import { DraftMark } from "@/components/chrome/draft-mark"
import { EmptyLine, Section } from "@/components/chrome/section"
import { StatusDot } from "@/components/chrome/status-dot"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { connectionsQuery, nodesQuery } from "@/features/network/queries"
import { freshnessLabel } from "@/features/overview/labels"
import { useDraftCommit } from "@/features/shell/draft-commit"
import { runtimeFor, runtimeFreshness } from "@/features/topology/runtime"
import { authed } from "@/lib/api"
import { relativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { discoveryFor, discoveryStatus, reportUsable } from "./discovery-model"
import { useListenerDraft } from "./drafts"
import {
  discoveryStatusLabel,
  listenerStateLabel,
  reasonLabel,
  runtimeStateLabel,
} from "./labels"
import {
  LISTENER_GRID,
  ListenerForm,
  listenerProblemText,
  presetLabel,
} from "./listener-form"
import {
  certificateText,
  endpointText,
  fieldsOf,
  listenerProblems,
  listenersOf,
  newListener,
  presetOf,
  presetsFor,
  settingsOf,
  type ListenerFields,
} from "./listener-model"
import type { NodeContext } from "./node-facts"
import { NodeSockets } from "./node-sockets"
import { discoveriesQuery, managedListenersQuery } from "./queries"
import { NEW_LISTENER } from "./url"

/**
 * A node's transport: the listeners the platform manages for it (edited in
 * place), then what the node reports about them and about its actual
 * sockets. Saved intent, the agent's report and the observed candidates
 * are separate facts with their own freshness.
 */
export function NodeTransport({
  networkId,
  node,
  context,
  canManage,
  editing,
  onEdit,
}: {
  networkId: string
  node: NodeSummary
  context: NodeContext
  canManage: boolean
  /** The listener whose editor is open (`NEW_LISTENER` for one being added). */
  editing: string | null
  onEdit: (listener: string | null) => void
}) {
  const listeners = useQuery(managedListenersQuery(networkId))
  const connections = useQuery(connectionsQuery(networkId))
  const discoveries = useQuery(discoveriesQuery(networkId))
  const mine = listenersOf(listeners.data, node.id)
  const active = node.revoked_at === null
  const manage = canManage && active

  const taken = useMemo(
    () =>
      new Set([
        ...(listeners.data?.listeners.map((listener) => listener.settings.id) ??
          []),
        ...(connections.data?.configuration.listeners.map(
          (listener) => listener.id
        ) ?? []),
      ]),
    [listeners.data, connections.data]
  )

  const report = runtimeFor(context.runtimes, node.id)
  const freshness = runtimeFreshness(report, context.now)
  const discovery = discoveryFor(discoveries.data, node.id)
  const status = discoveries.data ? discoveryStatus(discovery) : null

  return (
    <div className="flex flex-col gap-8">
      <div className="grid grid-cols-2 gap-2 xl:grid-cols-3">
        <Tile label={m.node_report()}>
          {freshnessLabel[freshness]()}
          {report?.reported_at ? (
            <span className="font-normal text-muted-foreground">
              {" "}
              · {relativeTime(report.reported_at, context.now)}
            </span>
          ) : null}
        </Tile>
        <Tile
          label={m.node_runtime_state()}
          tone={report?.state === "error" ? "warning" : undefined}
        >
          {report ? (
            <>
              {runtimeStateLabel[report.state]()}
              {report.reason !== "none" ? (
                <span className="font-normal text-muted-foreground">
                  {" "}
                  · {reasonLabel[report.reason]()}
                </span>
              ) : null}
            </>
          ) : (
            m.node_unknown()
          )}
        </Tile>
        <Tile label={m.node_discovery()}>
          {status ? discoveryStatusLabel[status]() : m.node_unknown()}
          {discovery?.reported_at ? (
            <span className="font-normal text-muted-foreground">
              {" "}
              · {relativeTime(discovery.reported_at, context.now)}
            </span>
          ) : null}
        </Tile>
      </div>

      <Section
        title={m.node_listeners()}
        count={listeners.data ? mine.length : null}
        action={
          manage && listeners.data ? (
            <Button
              variant="ghost"
              size="sm"
              className="h-8 rounded-full px-3"
              disabled={editing === NEW_LISTENER}
              onClick={() => onEdit(NEW_LISTENER)}
            >
              <PlusIcon />
              {m.node_listener_add()}
            </Button>
          ) : null
        }
      >
        {editing === NEW_LISTENER && listeners.data && manage ? (
          <div className="rounded-tile bg-fill-subtle p-3.5">
            <ListenerEditor
              key={`${node.id}/${NEW_LISTENER}`}
              networkId={networkId}
              node={node}
              summary={null}
              snapshot={listeners.data}
              taken={taken}
              canManage={manage}
              onClose={() => onEdit(null)}
              onSaved={(id) => onEdit(id)}
            />
          </div>
        ) : null}
        {!listeners.data ? (
          listeners.isError ? (
            <EmptyLine>{m.node_listeners_failed()}</EmptyLine>
          ) : (
            <Skeleton className="h-16 w-full rounded-tile" />
          )
        ) : mine.length === 0 && editing !== NEW_LISTENER ? (
          <EmptyLine>{m.node_listeners_none()}</EmptyLine>
        ) : (
          <ul className="flex flex-col gap-2">
            {mine.map((summary) => (
              <ListenerItem
                key={summary.settings.id}
                networkId={networkId}
                node={node}
                summary={summary}
                snapshot={listeners.data}
                connections={connections.data ?? null}
                context={context}
                open={editing === summary.settings.id}
                canManage={manage}
                taken={taken}
                onToggle={() =>
                  onEdit(
                    editing === summary.settings.id ? null : summary.settings.id
                  )
                }
              />
            ))}
          </ul>
        )}
      </Section>

      <Section
        title={m.node_sockets()}
        count={
          discovery && reportUsable(discoveryStatus(discovery))
            ? discovery.sockets.length
            : undefined
        }
      >
        <NodeSockets
          listeners={mine}
          connections={connections.data ?? null}
          discoveryRead={discoveries.data !== undefined}
          discoveryFailed={discoveries.isError && !discoveries.data}
          context={context}
          nodeId={node.id}
          discovery={discovery}
        />
      </Section>
    </div>
  )
}

/** What the node's report says about one listener; the report's own freshness first. */
function useListenerRuntime(
  context: NodeContext,
  nodeId: string,
  listenerId: string
) {
  const report = runtimeFor(context.runtimes, nodeId)
  const freshness = runtimeFreshness(report, context.now)
  const entry =
    freshness === "fresh"
      ? report?.listeners.find(
          (listener) => listener.listener_id === listenerId
        )
      : undefined
  return { freshness, entry }
}

function ListenerItem({
  networkId,
  node,
  summary,
  snapshot,
  connections,
  context,
  open,
  canManage,
  taken,
  onToggle,
}: {
  networkId: string
  node: NodeSummary
  summary: ManagedListenerSummary
  snapshot: ManagedListenersSnapshot
  connections: ConnectionsSnapshot | null
  context: NodeContext
  open: boolean
  canManage: boolean
  taken: ReadonlySet<string>
  onToggle: () => void
}) {
  const settings = summary.settings
  const key = `${node.id}/${settings.id}`
  const draft = useListenerDraft(networkId, key, undefined)
  const { freshness, entry } = useListenerRuntime(context, node.id, settings.id)
  const candidates =
    connections?.configuration.candidates.filter(
      (candidate) => candidate.listener_id === settings.id
    ).length ?? null

  let state: { text: string; color: string; lit: boolean }
  if (!settings.enabled)
    state = {
      text: m.node_listener_disabled(),
      color: "var(--muted-foreground)",
      lit: false,
    }
  else if (!summary.compiled_capability_available)
    state = {
      text: m.node_listener_awaiting_capability(),
      color: "var(--warning)",
      lit: false,
    }
  else if (freshness !== "fresh")
    state = {
      text: freshnessLabel[freshness](),
      color: "var(--muted-foreground)",
      lit: false,
    }
  else if (!entry)
    state = {
      text: m.node_listener_unreported(),
      color: "var(--muted-foreground)",
      lit: false,
    }
  else
    state =
      entry.state === "bound"
        ? {
            text: listenerStateLabel.bound(),
            color: "var(--vitality-live)",
            lit: true,
          }
        : {
            text:
              entry.reason === "none" || entry.reason === "bind_failed"
                ? listenerStateLabel.error()
                : `${listenerStateLabel.error()} · ${reasonLabel[entry.reason]()}`,
            color: "var(--destructive)",
            lit: false,
          }

  const endpoints = settings.advertised_endpoints
  return (
    <li className="rounded-tile bg-fill-subtle">
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggle}
        className="list-row gap-3 rounded-tile px-3.5 py-2.5"
      >
        <span className="flex size-4 shrink-0 items-center justify-center">
          <StatusDot color={state.color} shape={state.lit ? "live" : "solid"} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span className="truncate font-mono text-row font-medium">
              {settings.id}
            </span>
            <span className="shrink-0 text-meta text-muted-foreground">
              {presetLabel[presetOf(settings)]()}
            </span>
            {draft.dirty ? <DraftMark variant="text" /> : null}
          </span>
          <span className="mt-0.5 block truncate font-mono text-meta text-muted-foreground">
            {endpointText(settings.bind_address, settings.bind_port)}
            {" → "}
            {endpoints.length === 0
              ? m.node_listener_advertised_none()
              : endpoints.length === 1 && endpoints[0]
                ? endpointText(endpoints[0].host, endpoints[0].port)
                : m.node_listener_advertised_count({
                    count: String(endpoints.length),
                  })}
          </span>
        </span>
        <span className="shrink-0 text-meta text-muted-foreground">
          {state.text}
        </span>
        <ChevronDownIcon
          className="size-4 shrink-0 text-muted-foreground transition-transform duration-200 data-[open=true]:rotate-180"
          data-open={open}
        />
      </button>
      {open ? (
        <div className="swap-in flex flex-col gap-6 px-3.5 pt-2 pb-3.5">
          <div className={cn(LISTENER_GRID, "gap-y-2")}>
            <Tile label={m.node_listener_material()}>
              <span className="font-mono">
                v{String(summary.material_version)}
              </span>
            </Tile>
            <Tile label={m.node_listener_certificate()}>
              <span className="font-mono font-normal">
                {summary.certificate_sha256
                  ? certificateText(summary.certificate_sha256)
                  : "—"}
              </span>
            </Tile>
            <Tile
              label={m.node_listener_capability()}
              tone={
                summary.compiled_capability_available ? undefined : "warning"
              }
            >
              {summary.compiled_capability_available
                ? m.node_listener_capability_available()
                : m.node_listener_awaiting_capability()}
            </Tile>
            <Tile label={m.node_listener_candidates()}>
              <span className="tabular-nums">
                {candidates === null ? m.node_unknown() : String(candidates)}
              </span>
            </Tile>
          </div>
          <ListenerEditor
            key={key}
            networkId={networkId}
            node={node}
            summary={summary}
            snapshot={snapshot}
            taken={taken}
            canManage={canManage}
            onClose={onToggle}
            onSaved={() => undefined}
          />
        </div>
      ) : null}
    </li>
  )
}

const BLANK: ListenerFields = {
  id: "",
  preset: "vless_tcp",
  enabled: true,
  bindAddress: "",
  bindPort: "",
  endpoints: [],
  serverName: "",
  websocketPath: "",
  websocketHost: "",
}

/**
 * One listener's form, as its own draft against the revision it was read
 * at. A new listener starts as a draft holding the suggested values;
 * discarding it closes the editor.
 */
function ListenerEditor({
  networkId,
  node,
  summary,
  snapshot,
  taken,
  canManage,
  onClose,
  onSaved,
}: {
  networkId: string
  node: NodeSummary
  summary: ManagedListenerSummary | null
  snapshot: ManagedListenersSnapshot
  taken: ReadonlySet<string>
  canManage: boolean
  onClose: () => void
  onSaved: (id: string) => void
}) {
  const queryClient = useQueryClient()
  const creating = summary === null
  const key = `${node.id}/${summary?.settings.id ?? NEW_LISTENER}`
  const saved = useMemo(
    () => ({
      revision: snapshot.revision,
      value: summary ? fieldsOf(summary.settings) : BLANK,
    }),
    [snapshot.revision, summary]
  )
  const draft = useListenerDraft(networkId, key, saved)
  const fields = draft.value ?? saved.value
  const creatingContext = useMemo(
    () => (creating ? { taken } : null),
    [creating, taken]
  )
  const problems = canManage ? listenerProblems(fields, creatingContext) : []
  const first = problems[0]

  // A new listener starts as a draft of the suggested values; once it has
  // been a draft, ending it (discarded, not saved) closes the editor.
  const wasDirty = useRef(false)
  const finished = useRef(false)
  const { dirty, edit } = draft
  useEffect(() => {
    if (creating && !dirty && !wasDirty.current)
      edit(() => newListener("vless_tcp", taken))
  }, [creating, dirty, edit, taken])
  useEffect(() => {
    if (dirty) wasDirty.current = true
    else if (creating && wasDirty.current && !finished.current) {
      finished.current = true
      onClose()
    }
  }, [creating, dirty, onClose])

  const listenersKey = managedListenersQuery(networkId).queryKey
  const commit = useDraftCommit({
    page: "nodes",
    draft,
    saved,
    problem: first ? listenerProblemText[first.kind]() : null,
    save: (value, revision) =>
      authed((base, token) =>
        saveManagedListener(base, token, networkId, {
          expected_revision: revision,
          settings: settingsOf(value, node.id),
        })
      ),
    onSaved: (next) => {
      finished.current = true
      queryClient.setQueryData(listenersKey, next)
      void queryClient.invalidateQueries({
        queryKey: connectionsQuery(networkId).queryKey,
      })
      void queryClient.invalidateQueries({
        queryKey: nodesQuery(networkId).queryKey,
      })
      if (creating) onSaved(fields.id.trim())
    },
    refresh: () =>
      void queryClient.invalidateQueries({ queryKey: listenersKey }),
  })

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault()
        commit.submit()
      }}
    >
      <button type="submit" hidden />
      <ListenerForm
        idPrefix={`listener-${summary?.settings.id ?? "new"}`}
        fields={fields}
        presets={presetsFor(summary?.settings ?? null)}
        creating={creatingContext}
        problems={problems}
        disabled={!canManage}
        showEnabled={!creating}
        onChange={(next) => draft.edit(() => next)}
      />
    </form>
  )
}
