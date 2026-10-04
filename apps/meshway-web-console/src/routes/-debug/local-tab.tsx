import { useQueryClient } from "@tanstack/react-query"
import { useStore } from "zustand"

import { observationsKey, subscriptionQuery } from "@/api/queries/observation"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { useNow } from "@/hooks/use-now"
import { isDirty, type DraftEntry } from "@/lib/configuration/draft-state"
import { dateTime, relativeTime, shortId } from "@/lib/format"
import { connectionWords } from "@/lib/live-labels"
import { leasePlan } from "@/lib/observation/lease"
import type { ObservationRecord } from "@/lib/observation/record"
import { m } from "@/paraglide/messages.js"
import { unsettledAttempts } from "@/state/attempts"
import { useBootPhase } from "@/state/boot"
import { useApplyStates, type ApplyState } from "@/state/configuration-apply"
import { useDrafts } from "@/state/configuration-draft"
import { useConnections } from "@/state/live"
import { sessionStore } from "@/state/session"

import { DataTable } from "./data-table"
import { duration, shortIds, traceTime } from "./format"

/**
 * What this tab holds outside the query cache: the sign-in, the drafts
 * (never their text), the apply flows, attempts whose outcome is unknown,
 * the open network's observation leases and every stream it runs.
 */
export function LocalTab({ networkId }: { networkId: string | null }) {
  const now = useNow(1_000)
  return (
    <div className="flex flex-col gap-6">
      <SessionFacts now={now} />
      <Drafts />
      <Applies />
      <Attempts />
      {networkId ? <Observations networkId={networkId} now={now} /> : null}
      <Streams />
    </div>
  )
}

function SessionFacts({ now }: { now: number }) {
  const { session, rejectedAt } = useStore(sessionStore, (state) => state)
  const boot = useBootPhase()
  return (
    <Section title={m.debug_session()}>
      <div className="grid grid-cols-2 gap-2 md:grid-cols-4">
        <Tile label={m.debug_session_id()}>
          <span className="font-mono font-normal">
            {session ? shortId(session.id) : "—"}
          </span>
        </Tile>
        <Tile label={m.debug_session_expires()}>
          {session
            ? `${dateTime(session.expiresAt) ?? "—"} · ${relativeTime(session.expiresAt, now) ?? ""}`
            : "—"}
        </Tile>
        <Tile label={m.debug_session_rejected()}>
          {rejectedAt === null ? "—" : traceTime(rejectedAt)}
        </Tile>
        <Tile label={m.debug_boot()}>
          <span className="font-mono font-normal">{boot}</span>
        </Tile>
      </div>
    </Section>
  )
}

function draftCells(entry: DraftEntry) {
  const { input, checked } = entry
  return [
    input.kind === "document"
      ? m.debug_draft_document()
      : m.debug_draft_operations({ count: input.ops.length }),
    m.revision({ revision: entry.baseRevision }),
    isDirty(entry) ? m.debug_draft_dirty() : m.debug_draft_clean(),
    checked
      ? m.debug_draft_checked({ count: checked.issues.length })
      : m.debug_draft_unchecked(),
  ]
}

function Drafts() {
  const drafts = Object.entries(useDrafts())
  return (
    <Section title={m.debug_drafts()} count={drafts.length}>
      {drafts.length === 0 ? (
        <EmptyLine>{m.debug_none()}</EmptyLine>
      ) : (
        <DataTable
          columns={[
            { label: m.debug_network() },
            { label: m.debug_draft_input() },
            { label: m.debug_draft_base() },
            { label: m.debug_draft_edits() },
            { label: m.debug_draft_check() },
          ]}
          rows={drafts.map(([networkId, entry]) => ({
            key: networkId,
            cells: [shortId(networkId), ...draftCells(entry)],
          }))}
        />
      )}
    </Section>
  )
}

const busyLabel = (busy: ApplyState["busy"]) =>
  busy === "checking"
    ? m.debug_apply_checking()
    : busy === "applying"
      ? m.debug_apply_applying()
      : "—"

function Applies() {
  const flows = Object.entries(useApplyStates())
  return (
    <Section title={m.debug_applies()} count={flows.length}>
      {flows.length === 0 ? (
        <EmptyLine>{m.debug_none()}</EmptyLine>
      ) : (
        <DataTable
          columns={[
            { label: m.debug_network() },
            { label: m.debug_apply_busy() },
            { label: m.debug_apply_review() },
            { label: m.debug_apply_refusal() },
          ]}
          rows={flows.map(([networkId, flow]) => ({
            key: networkId,
            tone: flow.review?.unknown ? "warning" : undefined,
            cells: [
              shortId(networkId),
              busyLabel(flow.busy),
              flow.review
                ? `${m.revision({ revision: flow.review.proposal.expectedRevision })}${
                    flow.review.unknown ? ` · ${m.debug_apply_unknown()}` : ""
                  }`
                : "—",
              flow.refusal
                ? m.revision({
                    revision: flow.refusal.proposal.expectedRevision,
                  })
                : "—",
            ],
          }))}
        />
      )}
    </Section>
  )
}

/** Read each render (the panel's clock ticks): attempts are runtime, not state. */
function Attempts() {
  const unsettled = unsettledAttempts()
  return (
    <Section title={m.debug_attempts()} count={unsettled.length}>
      {unsettled.length === 0 ? (
        <EmptyLine>{m.debug_none()}</EmptyLine>
      ) : (
        <DataTable
          columns={[
            { label: m.debug_attempt_kind() },
            { label: m.debug_attempt_scopes() },
          ]}
          rows={unsettled.map(({ kind, scopes }) => ({
            key: kind,
            tone: "warning",
            cells: [
              kind,
              scopes.map((scope) => (scope ? shortIds(scope) : "—")).join(", "),
            ],
          }))}
        />
      )}
    </Section>
  )
}

function leaseText(record: ObservationRecord, now: number) {
  const task = record.task
  if (!task || record.removed) return m.debug_observation_removed()
  const plan = leasePlan(task, now)
  switch (plan.type) {
    case "renew":
      return m.debug_observation_renew({ time: duration(plan.inMs) })
    case "lapsed":
      return m.debug_observation_lapsed()
    case "none":
      return "—"
  }
}

function Observations({ networkId, now }: { networkId: string; now: number }) {
  const queryClient = useQueryClient()
  const records = queryClient
    .getQueryCache()
    .findAll({ queryKey: [...observationsKey(networkId), "subscription"] })
    .flatMap(({ queryKey }) => {
      const subscriptionId = queryKey[4]
      if (typeof subscriptionId !== "string") return []
      const record = queryClient.getQueryData(
        subscriptionQuery(queryClient, networkId, subscriptionId).queryKey
      )
      return record ? [{ subscriptionId, record }] : []
    })
  return (
    <Section title={m.debug_observations()} count={records.length}>
      {records.length === 0 ? (
        <EmptyLine>{m.debug_none()}</EmptyLine>
      ) : (
        <DataTable
          columns={[
            { label: m.debug_observation_id() },
            { label: m.debug_observation_phase() },
            { label: m.debug_observation_lease() },
            { label: m.debug_observation_expires() },
            { label: m.debug_observation_plan() },
          ]}
          rows={records.map(({ subscriptionId, record }) => {
            const task = record.task
            const at = (time: string | null | undefined) =>
              time ? (relativeTime(time, now) ?? "—") : "—"
            return {
              key: subscriptionId,
              cells: [
                shortId(subscriptionId),
                task?.phase ?? "—",
                at(task?.subscriberExpiresAt),
                at(task?.expiresAt),
                leaseText(record, now),
              ],
            }
          })}
        />
      )}
    </Section>
  )
}

function Streams() {
  const connections = Object.entries(useConnections())
  return (
    <Section title={m.debug_streams()} count={connections.length}>
      {connections.length === 0 ? (
        <EmptyLine>{m.debug_none()}</EmptyLine>
      ) : (
        <DataTable
          columns={[
            { label: m.debug_network() },
            { label: m.debug_connection() },
            { label: m.debug_since() },
          ]}
          rows={connections.map(([networkId, connection]) => ({
            key: networkId,
            cells: [
              shortId(networkId),
              connectionWords(connection).title,
              traceTime(connection.since),
            ],
          }))}
        />
      )}
    </Section>
  )
}
