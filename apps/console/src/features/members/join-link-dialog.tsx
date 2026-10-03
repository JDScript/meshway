import {
  createJoinLink,
  revokeJoinLink,
  type CreateJoinLinkRequest,
  type JoinLinkCreated,
  type JoinLinkView,
  type MembershipRole,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import {
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"

import { BusyLabel } from "@/components/chrome/busy-label"
import { GlassDialog, SecretField } from "@/components/chrome/glass-dialog"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { fragmentLink } from "@/features/entry/fragment"
import { roleLabel, ROLES } from "@/features/network/roles"
import { useUnmountAfter } from "@/hooks/use-kept"
import { useSecretMutation } from "@/hooks/use-secret-mutation"
import { ApiFailure, authed, client } from "@/lib/api"
import { attemptFor, outcomeUnknown, type Attempt } from "@/lib/attempt"
import { toastFailure } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { JOIN_LINK_EXPIRIES, type JoinLinkExpiry } from "./join-link"
import { joinLinksQuery } from "./queries"

/** Long enough for the dialog to finish closing before its content (and the link) is dropped. */
const UNMOUNT_AFTER_MS = 400

type Body = Omit<CreateJoinLinkRequest, "requestId">

const expiryLabel: Record<JoinLinkExpiry, () => string> = {
  3_600: () => m.members_link_hour(),
  86_400: () => m.members_link_day(),
  259_200: () => m.members_link_days({ count: "3" }),
  604_800: () => m.members_link_days({ count: "7" }),
}

/**
 * Issues a join link: one role, one expiry. The link is shown once, in this
 * dialog only; it is dropped when the dialog closes, and the dialog itself
 * unmounts so nothing keeps it.
 */
export function JoinLinkDialog(props: {
  networkId: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const mounted = useUnmountAfter(props.open, UNMOUNT_AFTER_MS)
  return mounted ? <Issue {...props} /> : null
}

function Issue({
  networkId,
  open,
  onOpenChange,
}: {
  networkId: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()
  const formId = useId()
  const [role, setRole] = useState<MembershipRole>("member")
  const [expiry, setExpiry] = useState<JoinLinkExpiry>(86_400)
  const [replacing, setReplacing] = useState(false)
  // An attempt whose outcome is unknown is sent again as itself.
  const unsettled = useRef<Attempt<Body> | null>(null)

  const secret = useSecretMutation(
    async (attempt: Attempt<Body>): Promise<JoinLinkCreated> => {
      try {
        const created = await authed((token) =>
          createJoinLink(
            client,
            { networkId },
            { requestId: attempt.requestId, ...attempt.body },
            { token }
          )
        )
        unsettled.current = null
        return created
      } catch (error) {
        unsettled.current =
          error instanceof ApiFailure && outcomeUnknown(error.failure)
            ? attempt
            : null
        throw error
      }
    },
    { permission_denied: () => m.members_link_role_denied() }
  )
  const created = secret.result

  useEffect(() => {
    if (created)
      void queryClient.invalidateQueries({
        queryKey: joinLinksQuery(networkId).queryKey,
      })
  }, [created, queryClient, networkId])

  const close = (next: boolean) => {
    onOpenChange(next)
    if (!next) secret.clear()
  }

  const issue = (body: Body) =>
    secret.create(
      attemptFor(
        unsettled.current,
        `${body.role}:${body.expiresInSeconds}`,
        () => body
      )
    )

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (secret.pending) return
    issue({ role, expiresInSeconds: expiry })
  }

  // The controller answered a repeated request: the link exists, its value is gone.
  const replace = async (link: JoinLinkView) => {
    setReplacing(true)
    try {
      await authed((token) =>
        revokeJoinLink(client, { networkId, linkId: link.id }, { token })
      )
    } catch (error) {
      toastFailure(error)
      setReplacing(false)
      return
    }
    void queryClient.invalidateQueries({
      queryKey: joinLinksQuery(networkId).queryKey,
    })
    setReplacing(false)
    secret.clear()
    issue({ role: link.role, expiresInSeconds: expiry })
  }

  if (created) {
    const link =
      created.secretDelivery === "available"
        ? fragmentLink(window.location.origin, "join", created.secret)
        : null
    return (
      <GlassDialog
        open={open}
        onOpenChange={close}
        title={m.members_link_created()}
        busy={replacing}
        footer={
          link ? (
            <Button
              className="h-9 rounded-full px-4"
              onClick={() => close(false)}
            >
              {m.members_done()}
            </Button>
          ) : (
            <>
              <Button
                variant="ghost"
                className="h-9 rounded-full px-4"
                disabled={replacing}
                onClick={() => close(false)}
              >
                {m.members_done()}
              </Button>
              <Button
                className="h-9 rounded-full px-4"
                disabled={replacing}
                aria-busy={replacing}
                onClick={() => void replace(created.link)}
              >
                <BusyLabel busy={replacing}>
                  {m.members_link_reissue()}
                </BusyLabel>
              </Button>
            </>
          )
        }
      >
        <div className="grid grid-cols-2 gap-2">
          <Tile label={m.members_role()}>{roleLabel[created.link.role]()}</Tile>
          <Tile label={m.members_link_expires()}>
            {dateTime(created.link.expiresAt) ?? "—"}
          </Tile>
        </div>
        {link ? (
          <SecretField label={m.members_link()} value={link} />
        ) : (
          <p className="text-row text-muted-foreground">
            {m.members_link_unavailable()}
          </p>
        )}
      </GlassDialog>
    )
  }

  return (
    <GlassDialog
      open={open}
      onOpenChange={close}
      title={m.members_link_create()}
      busy={secret.pending}
      footer={
        <>
          <Button
            variant="ghost"
            className="h-9 rounded-full px-4"
            disabled={secret.pending}
            onClick={() => close(false)}
          >
            {m.dialog_cancel()}
          </Button>
          <Button
            type="submit"
            form={formId}
            className="h-9 rounded-full px-4"
            disabled={secret.pending}
            aria-busy={secret.pending}
          >
            <BusyLabel busy={secret.pending}>
              {m.members_link_create_submit()}
            </BusyLabel>
          </Button>
        </>
      }
    >
      <form
        id={formId}
        noValidate
        className="flex flex-col gap-5"
        onSubmit={submit}
      >
        <Choice label={m.members_role()}>
          <SegmentedControl<MembershipRole>
            label={m.members_role()}
            value={role}
            options={ROLES.map((value) => ({
              value,
              label: roleLabel[value](),
            }))}
            onChange={setRole}
          />
        </Choice>
        <Choice label={m.members_link_valid_for()}>
          <SegmentedControl<`${JoinLinkExpiry}`>
            label={m.members_link_valid_for()}
            value={`${expiry}`}
            options={JOIN_LINK_EXPIRIES.map((seconds) => ({
              value: `${seconds}`,
              label: expiryLabel[seconds](),
            }))}
            onChange={(value) => {
              const next = JOIN_LINK_EXPIRIES.find(
                (seconds) => `${seconds}` === value
              )
              if (next !== undefined) setExpiry(next)
            }}
          />
        </Choice>
      </form>
    </GlassDialog>
  )
}

function Choice({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col items-start gap-2">
      <span aria-hidden className="text-meta text-muted-foreground">
        {label}
      </span>
      {children}
    </div>
  )
}
