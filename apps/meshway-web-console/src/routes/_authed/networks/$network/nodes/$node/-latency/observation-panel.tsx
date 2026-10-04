import {
  cancelObservation,
  type ObservationTask,
} from "@meshway/control-client"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useMemo, useState } from "react"

import { authed, client } from "@/api/client"
import { failureText, toastFailure } from "@/api/failure"
import { sessionQuery } from "@/api/queries/auth"
import { networkQuery } from "@/api/queries/network"
import { nodesQuery } from "@/api/queries/nodes"
import { observationListQuery } from "@/api/queries/observation"
import { networkStateQuery } from "@/api/queries/state"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Button } from "@/components/ui/button"
import { useNow } from "@/hooks/use-now"
import { relativeTime } from "@/lib/format"
import { canObserveFrom, pathChoices } from "@/lib/observation/choices"
import { isActive } from "@/lib/observation/record"
import { m } from "@/paraglide/messages.js"

import { phaseLabel, serviceText } from "./labels"
import { LiveObservation } from "./live-view"
import { StartForm } from "./start-form"

/**
 * Active observation from one node: start one (where the controller lets
 * this account), follow the one this tab holds, and this sign-in's other
 * subscriptions from the node, to take over or cancel after a reload. It
 * measures only while the tab holds a subscription and is visible; it
 * shows only what the controller and the node reported.
 */
export function ObservationPanel({
  networkId,
  nodeId,
}: {
  networkId: string
  nodeId: string
}) {
  const [held, setHeld] = useState<string | null>(null)
  const network = useQuery(networkQuery(networkId))
  const session = useQuery(sessionQuery())
  const nodes = useQuery(nodesQuery(networkId))
  const state = useQuery(networkStateQuery(networkId))
  const list = useQuery(observationListQuery(networkId))

  const names = useMemo(
    () =>
      new Map(
        (nodes.data?.items ?? []).map((node) => [node.id, node.hostname])
      ),
    [nodes.data]
  )
  const nameOf = (id: string) => names.get(id) ?? id.slice(0, 8)
  const node = nodes.data?.items.find((item) => item.id === nodeId) ?? null
  const allowed =
    node !== null &&
    canObserveFrom(
      network.data?.network.role ?? null,
      session.data?.user.email ?? null,
      node.identity
    )

  const targets = useMemo(() => {
    const ids = new Set([
      ...(nodes.data?.items ?? []).map((item) => item.id),
      ...Object.keys(state.data?.nodes ?? {}),
    ])
    ids.delete(nodeId)
    return [...ids].sort((a, b) =>
      (names.get(a) ?? a).localeCompare(names.get(b) ?? b)
    )
  }, [nodes.data, state.data, names, nodeId])

  const mine = (list.data?.items ?? []).filter(
    (item) => item.sourceNodeId === nodeId
  )
  const pathsTo = (target: string) =>
    pathChoices(
      nodeId,
      target,
      state.data?.paths ?? [],
      mine.flatMap((item) =>
        item.targetNodeId === target && item.path ? [item.path] : []
      )
    )

  const others = mine.filter(
    (item) => item.subscriptionId !== held && isActive(item.phase)
  )

  return (
    <div className="flex flex-col gap-6 border-b border-divider pb-6">
      <Section title={m.observation_title()}>
        {held ? (
          <LiveObservation
            key={held}
            networkId={networkId}
            subscriptionId={held}
            nameOf={nameOf}
            onClose={() => setHeld(null)}
          />
        ) : nodes.data === undefined || network.data === undefined ? (
          <EmptyLine>{m.observation_reading()}</EmptyLine>
        ) : allowed ? (
          <StartForm
            networkId={networkId}
            sourceId={nodeId}
            targets={targets}
            pathsTo={pathsTo}
            nameOf={nameOf}
            onStarted={setHeld}
          />
        ) : (
          <EmptyLine>{m.observation_not_allowed()}</EmptyLine>
        )}
      </Section>

      {others.length || list.isError ? (
        <Section title={m.observation_list()} count={others.length}>
          {list.isError ? (
            <EmptyLine>
              {failureText(list.error) ?? m.observation_failed()}
            </EmptyLine>
          ) : (
            <div className="flex flex-col gap-0.5">
              {others.map((item) => (
                <OtherRow
                  key={item.subscriptionId ?? item.id}
                  networkId={networkId}
                  task={item}
                  nameOf={nameOf}
                  canTake={held === null}
                  onTake={() => setHeld(item.subscriptionId ?? null)}
                />
              ))}
            </div>
          )}
        </Section>
      ) : null}
    </div>
  )
}

/** A subscription of this sign-in this tab does not hold: another tab's, or one from before a reload. */
function OtherRow({
  networkId,
  task,
  nameOf,
  canTake,
  onTake,
}: {
  networkId: string
  task: ObservationTask
  nameOf: (id: string) => string
  canTake: boolean
  onTake: () => void
}) {
  const now = useNow(5_000)
  const queryClient = useQueryClient()
  const subscriptionId = task.subscriptionId ?? null
  const cancel = useMutation({
    mutationFn: (id: string) =>
      authed((token) =>
        cancelObservation(client, { networkId, subscriptionId: id }, { token })
      ),
    onSuccess: () =>
      void queryClient.invalidateQueries({
        queryKey: observationListQuery(networkId).queryKey,
      }),
    onError: toastFailure,
  })
  const active = isActive(task.phase)
  return (
    <div className="flex min-h-12 items-center gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-row">
          → {nameOf(task.targetNodeId)}
          <span className="text-muted-foreground">
            {" · "}
            {serviceText(task.service)}
          </span>
        </span>
        <span className="truncate text-meta text-muted-foreground">
          {[
            phaseLabel[task.phase](),
            active
              ? m.observation_lease_until({
                  time:
                    relativeTime(
                      task.subscriberExpiresAt ?? task.expiresAt,
                      now
                    ) ?? "—",
                })
              : null,
            subscriptionId?.slice(0, 8),
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </div>
      {active && canTake && subscriptionId ? (
        <Button variant="outline" size="sm" onClick={onTake}>
          {m.observation_take()}
        </Button>
      ) : null}
      {active && subscriptionId ? (
        <Button
          variant="ghost"
          size="sm"
          disabled={cancel.isPending}
          onClick={() => cancel.mutate(subscriptionId)}
        >
          {m.observation_cancel()}
        </Button>
      ) : null}
    </div>
  )
}
