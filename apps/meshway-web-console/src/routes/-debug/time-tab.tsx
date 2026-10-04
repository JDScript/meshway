import { useQueryClient } from "@tanstack/react-query"
import { useStore } from "zustand"

import { configurationQuery } from "@/api/queries/configuration"
import { networkStateQuery } from "@/api/queries/state"
import { Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { useNow } from "@/hooks/use-now"
import { clockOffset } from "@/lib/debug/clock"
import { agoTime, relativeTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"
import { useClockSamples, useStreamTrace } from "@/state/debug-trace"
import { sessionStore } from "@/state/session"

import { milliseconds } from "./format"

const GRID = "grid grid-cols-2 gap-2 md:grid-cols-4"

/**
 * The controller's clock against this browser's (from the `Date` of the
 * responses recorded), and the times the console reads against it: the
 * open network's State and the session's expiry, by either clock.
 */
export function TimeTab({ networkId }: { networkId: string | null }) {
  const now = useNow(1_000)
  const samples = useClockSamples()
  const offset = clockOffset(samples)
  const middle = offset ? Math.round((offset.low + offset.high) / 2) : null
  const serverNow = middle === null ? null : now + middle
  const session = useStore(sessionStore, (state) => state.session)

  return (
    <div className="flex flex-col gap-6">
      <Section title={m.debug_clock()}>
        <div className={GRID}>
          <Tile label={m.debug_clock_offset()}>
            {middle === null
              ? m.debug_clock_unmeasured()
              : middle >= 0
                ? m.debug_clock_ahead({ time: milliseconds(middle) })
                : m.debug_clock_behind({ time: milliseconds(-middle) })}
          </Tile>
          <Tile label={m.debug_clock_error()}>
            {offset
              ? `± ${milliseconds(Math.round((offset.high - offset.low) / 2))}`
              : "—"}
          </Tile>
          <Tile label={m.debug_clock_samples()}>
            {m.debug_clock_samples_used({
              used: offset?.samples ?? 0,
              total: samples.length,
            })}
          </Tile>
        </div>
      </Section>
      {networkId ? (
        <NetworkTimes networkId={networkId} now={now} serverNow={serverNow} />
      ) : null}
      <Section title={m.debug_session()}>
        <div className={GRID}>
          <Tile label={m.debug_expires_browser()}>
            {session ? (relativeTime(session.expiresAt, now) ?? "—") : "—"}
          </Tile>
          <Tile label={m.debug_expires_server()}>
            {session && serverNow !== null
              ? (relativeTime(session.expiresAt, serverNow) ?? "—")
              : "—"}
          </Tile>
        </div>
      </Section>
    </div>
  )
}

function NetworkTimes({
  networkId,
  now,
  serverNow,
}: {
  networkId: string
  now: number
  serverNow: number | null
}) {
  const queryClient = useQueryClient()
  const trace = useStreamTrace(networkId)
  const state = queryClient.getQueryData(networkStateQuery(networkId).queryKey)
  const configuration = queryClient.getQueryData(
    configurationQuery(networkId).queryKey
  )
  const revision = (value: number | null | undefined) =>
    value === null || value === undefined
      ? "—"
      : m.revision({ revision: value })
  return (
    <Section title={m.debug_state()}>
      <div className={GRID}>
        <Tile label={m.debug_as_of_browser()}>
          {state ? (agoTime(state.asOf, now) ?? "—") : "—"}
        </Tile>
        <Tile label={m.debug_as_of_server()}>
          {state && serverNow !== null
            ? (agoTime(state.asOf, serverNow) ?? "—")
            : "—"}
        </Tile>
        <Tile label={m.debug_sequence()}>{state?.sequence ?? "—"}</Tile>
        <Tile label={m.debug_epoch()}>
          <span className="font-mono font-normal">
            {state?.stateEpoch ?? "—"}
          </span>
        </Tile>
        <Tile label={m.debug_state_revision()}>
          {revision(state?.configurationRevision)}
        </Tile>
        <Tile label={m.debug_view_revision()}>{revision(trace.revision)}</Tile>
        <Tile label={m.debug_configuration_revision()}>
          {revision(configuration?.revision)}
        </Tile>
      </div>
    </Section>
  )
}
