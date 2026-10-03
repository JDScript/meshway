import {
  createAccountInvitation,
  type AccountRole,
  type CreateInvitationRequest,
} from "@meshway/control-client"
import { zCreateInvitationRequest } from "@meshway/control-client/schemas"
import { useQueryClient } from "@tanstack/react-query"
import {
  useEffect,
  useId,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"
import { toast } from "sonner"

import { TextField } from "@/components/chrome/form"
import { GlassDialog, SecretField } from "@/components/chrome/glass-dialog"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { useUnmountAfter } from "@/hooks/use-kept"
import { useSecretMutation } from "@/hooks/use-secret-mutation"
import { authed } from "@/lib/api"
import { formText, validEmail } from "@/lib/form"
import { unixDateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { invitationLink } from "./invitation"
import { invitationsQuery } from "./queries"
import { roleLabel } from "./roles"

const DAY = 86_400
/** The controller allows at most seven days. */
const EXPIRIES = ["1", "3", "7"] as const
type Expiry = (typeof EXPIRIES)[number]

/** Long enough for the dialog to finish closing before its content (and the link) is dropped. */
const UNMOUNT_AFTER_MS = 400

/**
 * Invites one email into the network with one role. The link is shown once,
 * in this dialog only: it is dropped when the dialog closes, and the dialog
 * itself unmounts so nothing keeps it.
 */
export function InviteDialog(props: {
  networkId: string
  roles: readonly AccountRole[]
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const mounted = useUnmountAfter(props.open, UNMOUNT_AFTER_MS)
  return mounted ? <Invite {...props} /> : null
}

function Invite({
  networkId,
  roles,
  open,
  onOpenChange,
}: {
  networkId: string
  roles: readonly AccountRole[]
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()
  const formId = useId()
  const [role, setRole] = useState<AccountRole>("member")
  const [expiry, setExpiry] = useState<Expiry>("3")
  const [invalid, setInvalid] = useState(false)
  const secret = useSecretMutation((body: CreateInvitationRequest) =>
    authed((base, token) =>
      createAccountInvitation(base, token, networkId, body)
    )
  )
  const created = secret.result
  const link = created ? invitationLink(created.invitation_token) : null

  useEffect(() => {
    if (!created) return
    void queryClient.invalidateQueries({
      queryKey: invitationsQuery(networkId).queryKey,
    })
    // Committed, but not a link this console can hand over.
    if (!invitationLink(created.invitation_token))
      toast.error(m.members_invite_link_unusable())
  }, [created, queryClient, networkId])

  const close = (next: boolean) => {
    onOpenChange(next)
    if (!next) secret.clear()
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (secret.pending) return
    const email = formText(new FormData(event.currentTarget), "email").trim()
    const body = zCreateInvitationRequest.safeParse({
      email,
      role,
      expires_in_seconds: Number(expiry) * DAY,
    })
    if (!body.success || !validEmail(email)) {
      setInvalid(true)
      toast.error(m.members_invite_email_invalid(), { id: "invite" })
      return
    }
    setInvalid(false)
    toast.dismiss("invite")
    secret.create(body.data)
  }

  if (created) {
    return (
      <GlassDialog
        open={open}
        onOpenChange={close}
        title={m.members_invite_created()}
        footer={
          <Button
            className="h-9 rounded-full px-4"
            onClick={() => close(false)}
          >
            {m.dialog_done()}
          </Button>
        }
      >
        <div className="grid grid-cols-2 gap-2">
          <div className="col-span-2">
            <Tile label={m.members_email()}>{created.invitation.email}</Tile>
          </div>
          <Tile label={m.members_role()}>
            {roleLabel[created.invitation.role]()}
          </Tile>
          <Tile label={m.members_invite_expires()}>
            {unixDateTime(created.invitation.expires_at) ?? "—"}
          </Tile>
        </div>
        {link ? (
          <SecretField label={m.members_invite_link()} value={link} />
        ) : null}
      </GlassDialog>
    )
  }

  return (
    <GlassDialog
      open={open}
      onOpenChange={close}
      title={m.members_invite_title()}
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
            {secret.pending ? <Spinner /> : null}
            {m.members_invite_create()}
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
        <TextField
          id="invite-email"
          placeholder={m.placeholder_email()}
          name="email"
          type="email"
          inputMode="email"
          label={m.members_email()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={254}
          autoFocus
          invalid={invalid}
          disabled={secret.pending}
          onChange={() => setInvalid(false)}
        />
        <Choice label={m.members_role()}>
          <SegmentedControl<AccountRole>
            label={m.members_role()}
            value={role}
            options={roles.map((value) => ({
              value,
              label: roleLabel[value](),
            }))}
            onChange={setRole}
          />
        </Choice>
        <Choice label={m.members_invite_expires_in()}>
          <SegmentedControl<Expiry>
            label={m.members_invite_expires_in()}
            value={expiry}
            options={EXPIRIES.map((days) => ({
              value: days,
              label: m.members_days({ count: days }),
            }))}
            onChange={setExpiry}
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
