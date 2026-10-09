import type { NodeInventoryResponse, StateView } from "@meshway/control-client"
import { cn } from "@meshway/ui/cn"
import type { UseQueryResult } from "@tanstack/react-query"
import { PlusIcon, SearchIcon } from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"

import { DraftMark } from "@/components/chrome/draft-mark"
import {
  ListNote,
  ListPlaceholders,
  ListRow,
  PanelList,
  useListKeys,
} from "@/components/chrome/list"
import { NodeStatusDot } from "@/components/chrome/node-status"
import { PanelAction, PanelHeader } from "@/components/chrome/panel-header"
import { surfaceTitleId } from "@/components/surface/page-frame"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Skeleton } from "@/components/ui/skeleton"
import { useNow } from "@/hooks/use-now"
import { operationsOf } from "@/lib/configuration/draft-state"
import { editsNode, isDeleted } from "@/lib/configuration/node-operations"
import { shownNodes } from "@/lib/nodes/model"
import { fullReport, stateOf } from "@/lib/nodes/state-model"
import { lastSeen, nodeStatus } from "@/lib/nodes/status"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import { identityText, statusText } from "./-labels"

/** Typing settles this long before it is written to the address. */
const SEARCH_SETTLE_MS = 150

/**
 * The list's search field, bound to the URL without ever setting the field
 * while someone types: the field is uncontrolled, nothing is written during
 * an IME composition, and a URL change made elsewhere (back / forward) is
 * copied in only if it is not the one this field wrote.
 */
function SearchField({
  value,
  onSettle,
}: {
  value: string
  onSettle: (value: string) => void
}) {
  const field = useRef<HTMLInputElement>(null)
  const composing = useRef(false)
  const written = useRef(value)
  const timer = useRef<number | undefined>(undefined)
  // The field's first value only: later ones are written in place.
  const [initial] = useState(value)

  useEffect(() => {
    if (value === written.current) return
    written.current = value
    if (field.current && !composing.current) field.current.value = value
  }, [value])
  useEffect(() => () => window.clearTimeout(timer.current), [])

  const settle = () => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      const next = field.current?.value ?? ""
      if (composing.current || next === written.current) return
      written.current = next
      onSettle(next)
    }, SEARCH_SETTLE_MS)
  }

  return (
    <InputGroup className="h-9 rounded-full border-transparent bg-fill-subtle dark:bg-fill-subtle">
      <InputGroupAddon className="pl-3">
        <SearchIcon />
      </InputGroupAddon>
      <InputGroupInput
        ref={field}
        type="search"
        name="node-search"
        autoComplete="off"
        maxLength={120}
        placeholder={m.nodes_search()}
        aria-label={m.nodes_search()}
        defaultValue={initial}
        onChange={() => {
          if (!composing.current) settle()
        }}
        onCompositionStart={() => {
          composing.current = true
        }}
        onCompositionEnd={() => {
          composing.current = false
          settle()
        }}
      />
    </InputGroup>
  )
}

/**
 * The nodes page's index: the nodes this account may see, by hostname,
 * with their identity, address and status (an offline one with when it was
 * last seen). The search lives in the URL; choosing a row opens its
 * workspace.
 */
export function NodeList({
  networkId,
  nodes,
  state,
  stateFailed,
  q,
  openId,
  canAdd,
  onQuery,
  onOpen,
  onAdd,
}: {
  networkId: string
  nodes: UseQueryResult<NodeInventoryResponse>
  state: StateView | undefined
  /** The State failed to read: statuses are unknown, not loading. */
  stateFailed: boolean
  q: string
  openId: string | null
  canAdd: boolean
  onQuery: (q: string) => void
  onOpen: (nodeId: string) => void
  onAdd: () => void
}) {
  const all = nodes.data?.items
  const rows = useMemo(() => shownNodes(all ?? [], q), [all, q])
  const unread = all === undefined
  const now = useNow(1_000)
  const ops = operationsOf(useConfigurationDraft(networkId).entry)
  const { ref: list, onKeyDown } = useListKeys({
    ids: rows.map((node) => node.id),
    openId,
    onOpen,
  })

  // Keep the open row in view, wherever it was opened from.
  useEffect(() => {
    if (!openId) return
    list.current
      ?.querySelector(`[data-row-id="${CSS.escape(openId)}"]`)
      ?.scrollIntoView({ block: "nearest" })
  }, [list, openId])

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader
        page="nodes"
        title={m.nodes_title()}
        count={
          unread
            ? null
            : rows.length === all.length
              ? all.length
              : `${rows.length} / ${all.length}`
        }
        action={
          canAdd ? (
            <PanelAction
              label={m.nodes_add()}
              icon={PlusIcon}
              onClick={onAdd}
            />
          ) : undefined
        }
      >
        <SearchField value={q} onSettle={onQuery} />
      </PanelHeader>

      <PanelList
        ref={list}
        aria-labelledby={surfaceTitleId("nodes", "panel")}
        onKeyDown={onKeyDown}
      >
        {unread && !nodes.isError ? <ListPlaceholders rows={5} /> : null}
        {unread && nodes.isError ? (
          <ListNote>{m.nodes_failed()}</ListNote>
        ) : null}
        {!unread && rows.length === 0 ? (
          <ListNote>
            {all.length === 0 ? m.nodes_empty() : m.nodes_no_match()}
          </ListNote>
        ) : null}
        {rows.map((node) => {
          const view = stateOf(state, node.id)
          const report = fullReport(view)
          const status = nodeStatus(node, view)
          const stopped = status === "disabled" || status === "revoked"
          const deleting = isDeleted(ops, node.id)
          return (
            <ListRow
              key={node.id}
              rowId={node.id}
              current={node.id === openId}
              onClick={() => onOpen(node.id)}
              lead={<NodeStatusDot status={status} />}
              title={
                <>
                  <span
                    className={cn(
                      "truncate",
                      (stopped || deleting) && "text-muted-foreground",
                      deleting && "line-through"
                    )}
                  >
                    {node.hostname}
                  </span>
                  {editsNode(ops, node.id) ? <DraftMark /> : null}
                </>
              }
              subtitle={
                <>
                  <span className="shrink-0 font-mono">
                    {node.addresses.ipv4 ?? node.addresses.ipv6 ?? "—"}
                  </span>
                  <span className="min-w-0 truncate">
                    {identityText(node.identity)}
                    {report
                      ? ` · ${report.system.os} ${report.system.agentVersion}`
                      : ""}
                  </span>
                </>
              }
              trail={
                state !== undefined ? (
                  statusText(status, lastSeen(view), now)
                ) : stateFailed ? (
                  "—"
                ) : (
                  <Skeleton className="h-2.5 w-8 rounded-full" />
                )
              }
            />
          )
        })}
      </PanelList>
    </div>
  )
}
