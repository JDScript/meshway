import {
  revokeEnrollmentKey,
  type EnrollmentKeyView,
  type ListenerView,
  type NetworkSummary,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query"
import { KeyRoundIcon, XIcon } from "lucide-react"
import { useEffect, useMemo, useState, type ReactNode } from "react"
import { toast } from "sonner"

import { authed, client, codeOf } from "@/api/client"
import { failureText, toastFailureWith } from "@/api/failure"
import { membersQuery } from "@/api/queries/members"
import { enrollmentKeysQuery } from "@/api/queries/nodes"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { EmptyLine, Section } from "@/components/chrome/section"
import { useNow } from "@/hooks/use-now"
import { dateTime } from "@/lib/format"
import { presetOfView } from "@/lib/nodes/listener-edit"
import { endpointText } from "@/lib/nodes/listener-model"
import { enrollmentKeyState } from "@/lib/nodes/model"
import { m } from "@/paraglide/messages.js"

import { presetLabel, protocolLabel } from "../-labels"

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

/** At most this many pages are read to find the keys still usable (newest first). */
const PAGES = 10

/**
 * The enrolment keys that can still enrol a device (not used, revoked or
 * expired), newest first: what each enrols (tags, hostname, listeners),
 * who created it and until when, each revocable. Used and past keys are
 * not listed; the activity has them. Nothing is shown when there is none.
 */
export function EnrollmentKeys({
  networkId,
  network,
}: {
  networkId: string
  network: NetworkSummary | null
}) {
  const queryClient = useQueryClient()
  const keys = useInfiniteQuery(enrollmentKeysQuery(networkId))
  // Usable keys can be anywhere in the list: read it through, a bounded number of pages.
  const { hasNextPage, isFetchingNextPage, isError, fetchNextPage } = keys
  const read = keys.data?.pages.length ?? 0
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage && !isError && read < PAGES)
      void fetchNextPage()
  }, [hasNextPage, isFetchingNextPage, isError, read, fetchNextPage])
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
  const usable = useMemo(
    () =>
      (pages ?? [])
        .flatMap((page) => page.items)
        .filter((key) => enrollmentKeyState(key, now).state === "pending"),
    [pages, now]
  )

  let body: ReactNode
  if (pages === undefined) {
    // Unread: only a failure is said; nothing waits in its place.
    if (!keys.isError) return null
    body = (
      <EmptyLine>
        {failureText(keys.error) ?? m.nodes_enrollment_failed()}
      </EmptyLine>
    )
  } else if (usable.length === 0) return null
  else
    body = (
      <ul className="flex flex-col gap-0.5">
        {usable.map((key) => {
          const listeners = Object.entries(key.initialSettings.listeners).sort(
            ([a], [b]) => a.localeCompare(b)
          )
          return (
            <li
              key={key.id}
              className="flex min-h-12 w-full items-start gap-3 rounded-tile py-2 pr-2 pl-3.5"
            >
              <span className="flex h-5 w-4 shrink-0 items-center justify-center">
                <KeyRoundIcon className="size-4" />
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
                    m.nodes_enrollment_expires({
                      at: dateTime(key.expiresAt) ?? "—",
                    }),
                  ].join(" · ")}
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
      </ul>
    )

  return (
    <Section title={m.nodes_enrollment_usable()} count={usable.length || null}>
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
