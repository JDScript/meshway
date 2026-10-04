import type { EndpointCandidate, SocketState } from "@meshway/control-client"

import { relativeTime } from "@/lib/format"
import { candidateExpired, listenerRuntime } from "@/lib/nodes/state-model"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import {
  endpointSourceText,
  listenerPhaseLabel,
  problemText,
} from "../../-state-labels"

export function Runtime({
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

export function SocketBlock({
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
