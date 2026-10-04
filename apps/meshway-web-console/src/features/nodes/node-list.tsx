import type { NodeInventoryResponse, StateView } from "@meshway/control-client"
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
import { PanelAction, PanelHeader } from "@/components/chrome/panel-header"
import { StatusDot } from "@/components/chrome/status-dot"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { operationsOf } from "@/features/configuration/draft-state"
import { editsNode, isDeleted } from "@/features/configuration/node-operations"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { useNow } from "@/hooks/use-now"
import { relativeTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import { identityText } from "./labels"
import { keyRevoked, shownNodes } from "./model"
import { freshnessLabel } from "./state-labels"
import { fullReport, reportTime, stateOf, vitalityOf } from "./state-model"

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
 * with their identity, address, what stops them (disabled, a revoked
 * key) and their runtime report: lit while the controller holds a current
 * one, with when it arrived; else its freshness. The search lives in the
 * URL; choosing a row opens its workspace.
 */
export function NodeList({
  networkId,
  nodes,
  state,
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
  const now = useNow(15_000)
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
          const stopped = !node.enabled || keyRevoked(node)
          const view = stateOf(state, node.id)
          const report = fullReport(view)
          const at = reportTime(view)
          const deleting = isDeleted(ops, node.id)
          const fact =
            view === undefined
              ? undefined
              : view.freshness === "current" && at !== null
                ? (relativeTime(at, now) ?? freshnessLabel.current())
                : freshnessLabel[view.freshness]()
          return (
            <ListRow
              key={node.id}
              rowId={node.id}
              current={node.id === openId}
              onClick={() => onOpen(node.id)}
              lead={
                vitalityOf(view?.freshness) === "live" ? (
                  <StatusDot color="var(--vitality-live)" shape="live" />
                ) : (
                  <StatusDot color="var(--vitality-quiet)" shape="hollow" />
                )
              }
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
                keyRevoked(node)
                  ? m.nodes_key_revoked()
                  : !node.enabled
                    ? m.nodes_disabled()
                    : fact
              }
            />
          )
        })}
      </PanelList>
    </div>
  )
}
