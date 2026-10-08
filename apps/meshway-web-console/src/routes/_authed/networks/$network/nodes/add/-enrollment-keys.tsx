import {
  revokeEnrollmentKey,
  type EnrollmentKeyView,
  type ListenerView,
  type NetworkSummary,
  type NodeInventoryResponse,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query"
import { ArrowUpRightIcon, KeyRoundIcon, XIcon } from "lucide-react"
import { useMemo, useState, type ReactNode } from "react"
import { toast } from "sonner"

import { authed, client, codeOf } from "@/api/client"
import { failureText, toastFailureWith } from "@/api/failure"
import { membersQuery } from "@/api/queries/members"
import { enrollmentKeysQuery } from "@/api/queries/nodes"
import { BusyLabel } from "@/components/chrome/busy-label"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Skeleton } from "@/components/ui/skeleton"
import { useLoadMore } from "@/hooks/use-load-more"
import { useNow } from "@/hooks/use-now"
import { dateTime } from "@/lib/format"
import { presetOfView } from "@/lib/nodes/listener-edit"
import { endpointText } from "@/lib/nodes/listener-model"
import { enrollmentKeyState } from "@/lib/nodes/model"
import { m } from "@/paraglide/messages.js"

import { keyStateLabel, presetLabel, protocolLabel } from "../-labels"

/** A listener of a key's initial settings, as one line: its kind and where it is reached (else bound). */
function listenerLine(name: string, view: ListenerView) {
  const preset = presetOfView(view)
  const kind = preset ? presetLabel[preset]() : protocolLabel[view.protocol]()
  const endpoint = view.endpoints[0]
  return `${name} · ${kind} · ${
    endpoint
      ? endpointText(endpoint.host, endpoint.port)
      : endpointText(view.listen, view.listenPort)
  }`
}

/**
 * The network's enrolment keys, newest first as the controller pages them,
 * with its total: what each enrols (tags, hostname, listeners), who
 * created it, and its state; a used key names the node it became.
 */
export function EnrollmentKeys({
  networkId,
  network,
  nodes,
  onOpenNode,
}: {
  networkId: string
  network: NetworkSummary | null
  nodes: NodeInventoryResponse | undefined
  onOpenNode: (nodeId: string) => void
}) {
  const queryClient = useQueryClient()
  const keys = useInfiniteQuery(enrollmentKeysQuery(networkId))
  const loadMore = useLoadMore(keys)
  // The creators' emails, while this account may read the members.
  const members = useInfiniteQuery({
    ...membersQuery(networkId),
    enabled: network?.actions.manageMembers ?? false,
  })
  const emails = useMemo(
    () =>
      new Map(
        (members.data?.pages ?? []).flatMap((page) =>
          page.items.map((member) => [member.userId, member.email] as const)
        )
      ),
    [members.data]
  )
  const now = useNow(30_000)
  const [revoking, setRevoking] = useState<EnrollmentKeyView | null>(null)
  const [confirming, setConfirming] = useState(false)
  const revoke = useMutation({
    mutationFn: (keyId: string) =>
      authed((token) =>
        revokeEnrollmentKey(client, { networkId, keyId }, { token })
      ),
    onSuccess: () => {
      setConfirming(false)
      toast.success(m.nodes_enrollment_revoke_done())
    },
    onError: (error) => {
      setConfirming(false)
      // Used, expired or revoked before: the list read again shows which.
      if (codeOf(error) === "state_conflict")
        toast.warning(m.nodes_enrollment_revoke_moot())
      else
        toastFailureWith({ permission_denied: () => m.nodes_manage_denied() })(
          error
        )
    },
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: enrollmentKeysQuery(networkId).queryKey,
      }),
  })

  const pages = keys.data?.pages
  const rows = useMemo(
    () =>
      (pages ?? [])
        .flatMap((page) => page.items)
        .map((key) => ({ key, ...enrollmentKeyState(key, now) })),
    [pages, now]
  )
  const nodeName = (nodeId: string) =>
    nodes?.items.find((node) => node.id === nodeId)?.hostname ?? null

  let body: ReactNode
  if (pages === undefined)
    body = keys.isError ? (
      <EmptyLine>
        {failureText(keys.error) ?? m.nodes_enrollment_failed()}
      </EmptyLine>
    ) : (
      <Skeleton className="h-12 w-full rounded-tile" />
    )
  else if (rows.length === 0)
    body = <EmptyLine>{m.nodes_enrollment_none()}</EmptyLine>
  else
    body = (
      <ul className="flex flex-col gap-0.5">
        {rows.map(({ key, state, at }) => {
          const listeners = Object.entries(key.initialSettings.listeners).sort(
            ([a], [b]) => a.localeCompare(b)
          )
          const node = key.nodeId
          const name = node === null ? null : nodeName(node)
          const when =
            state === "consumed"
              ? at === null
                ? null
                : m.nodes_enrollment_consumed_at({ at: dateTime(at) ?? "—" })
              : state === "revoked"
                ? m.nodes_enrollment_revoked_at({
                    at: dateTime(at ?? "") ?? "—",
                  })
                : m.nodes_enrollment_expires({
                    at: dateTime(key.expiresAt) ?? "—",
                  })
          return (
            <li
              key={key.id}
              className="flex min-h-12 w-full items-start gap-3 rounded-tile py-2 pr-2 pl-3.5"
            >
              <span className="flex h-5 w-4 shrink-0 items-center justify-center">
                <KeyRoundIcon
                  className={cn(
                    "size-4",
                    state !== "pending" && "text-muted-foreground"
                  )}
                />
              </span>
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-row font-medium">
                  <span className="font-mono font-normal">
                    {key.tags.join(", ")}
                  </span>
                  {key.initialSettings.hostname !== null ? (
                    <span className="font-mono font-normal text-muted-foreground">
                      {" · "}
                      {key.initialSettings.hostname}
                    </span>
                  ) : null}
                </span>
                <span className="text-meta break-words text-muted-foreground">
                  {[
                    m.nodes_enrollment_created_by({
                      at: dateTime(key.createdAt) ?? "—",
                      user: emails.get(key.createdBy) ?? key.createdBy,
                    }),
                    when,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
                <span className="text-meta break-words text-muted-foreground">
                  {[
                    key.initialSettings.hostname === null
                      ? m.nodes_enrollment_hostname_device()
                      : null,
                    listeners.length === 0
                      ? m.nodes_enrollment_listeners_none()
                      : listeners
                          .map(([listener, view]) =>
                            listenerLine(listener, view)
                          )
                          .join("; "),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
                {node !== null ? (
                  name !== null ? (
                    <button
                      type="button"
                      onClick={() => onOpenNode(node)}
                      className="inline-flex items-center gap-1 self-start rounded-sm text-meta font-medium text-foreground underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      {m.nodes_enrollment_node({ name })}
                      <ArrowUpRightIcon className="size-3.5" />
                    </button>
                  ) : (
                    <span className="text-meta break-all text-muted-foreground">
                      {m.nodes_enrollment_node_gone({ id: node })}
                    </span>
                  )
                ) : null}
              </span>
              <span className="flex h-5 shrink-0 items-center text-meta text-muted-foreground">
                {keyStateLabel[state]()}
              </span>
              <span className="flex size-8 shrink-0 items-center justify-center">
                {key.actions.revoke ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    className="-mt-1.5 rounded-full"
                    aria-label={m.nodes_enrollment_revoke()}
                    onClick={() => {
                      setRevoking(key)
                      setConfirming(true)
                    }}
                  >
                    <XIcon />
                  </Button>
                ) : null}
              </span>
            </li>
          )
        })}
        {keys.hasNextPage ? (
          <li>
            <Button
              type="button"
              variant="ghost"
              className="h-10 w-full justify-start rounded-tile pl-3.5 text-muted-foreground"
              disabled={keys.isFetchingNextPage}
              onClick={loadMore}
            >
              <BusyLabel busy={keys.isFetchingNextPage}>
                {m.nodes_enrollment_more()}
              </BusyLabel>
            </Button>
          </li>
        ) : null}
      </ul>
    )

  return (
    <Section
      title={m.nodes_enrollment_keys()}
      count={pages ? (pages.at(-1)?.total ?? null) : null}
    >
      {body}
      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          if (!revoke.isPending) setConfirming(open)
        }}
        title={m.nodes_enrollment_revoke()}
        confirmLabel={m.nodes_enrollment_revoke()}
        pending={revoke.isPending}
        onConfirm={() => {
          if (revoking) revoke.mutate(revoking.id)
        }}
        effects={[
          m.nodes_enrollment_revoke_effect({
            tags: revoking ? revoking.tags.join(", ") : "",
          }),
          m.nodes_enrollment_revoke_effect_final(),
        ]}
      />
    </Section>
  )
}
