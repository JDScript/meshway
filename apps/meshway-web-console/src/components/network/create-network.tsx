import {
  createNetwork,
  type CreateNetworkRequest,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useRef, useState, type FormEvent } from "react"
import { toast } from "sonner"

import { authed, client } from "@/api/client"
import { failureText } from "@/api/failure"
import { networkQuery, networksQuery } from "@/api/queries/network"
import { BusyLabel } from "@/components/chrome/busy-label"
import { TextField, type FieldSize } from "@/components/chrome/form"
import { GlassDialog } from "@/components/chrome/glass-dialog"
import { Button } from "@/components/ui/button"
import { useNetworkNameCheck } from "@/hooks/use-network-name-check"
import { formText } from "@/lib/form"
import { m } from "@/paraglide/messages.js"
import { attempts } from "@/state/attempts"

const TOAST = "network-create"

/** One scope: a network is created under no other. */
const creations = attempts<Pick<CreateNetworkRequest, "name">>()

/** The words for a name the controller refuses (its rules may be newer than the page's). */
export const NAME_FAILURE = { invalid_request: () => m.network_name_invalid() }

/**
 * Creating a network. Submitting again while one is on its way does
 * nothing; after an attempt whose outcome is unknown, the same name is the
 * same request (its `requestId`), so the controller creates it once.
 */
export function useCreateNetwork(onCreated: (networkId: string) => void) {
  const queryClient = useQueryClient()
  const checkName = useNetworkNameCheck()
  const sending = useRef(false)
  const [pending, setPending] = useState(false)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (sending.current) return
    const checked = checkName(
      formText(new FormData(event.currentTarget), "name")
    )
    if ("refused" in checked) {
      toast.error(checked.refused, { id: TOAST })
      return
    }
    const { name } = checked
    sending.current = true
    setPending(true)
    try {
      const created = await creations.send(
        "",
        name,
        () => ({ name }),
        (attempt) =>
          authed((token) =>
            createNetwork(
              client,
              { requestId: attempt.requestId, ...attempt.body },
              { token }
            )
          )
      )
      queryClient.setQueryData(
        networkQuery(created.network.id).queryKey,
        created
      )
      void queryClient.invalidateQueries({
        queryKey: networksQuery().queryKey,
      })
      toast.dismiss(TOAST)
      onCreated(created.network.id)
    } catch (error) {
      const text = failureText(error, NAME_FAILURE)
      if (text) toast.error(text, { id: TOAST })
    } finally {
      sending.current = false
      setPending(false)
    }
  }

  return { pending, submit }
}

/** The name of a network to create. */
export function NetworkNameField({
  id,
  size,
  autoFocus,
}: {
  id: string
  size: FieldSize
  autoFocus?: boolean
}) {
  return (
    <TextField
      id={id}
      size={size}
      name="name"
      label={m.network_name()}
      placeholder={m.network_name_placeholder()}
      autoComplete="off"
      autoFocus={autoFocus}
    />
  )
}

/** "新建网络" from the network menu: a create too short for a workspace. */
export function CreateNetworkDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (networkId: string) => void
}) {
  const create = useCreateNetwork((networkId) => {
    onOpenChange(false)
    onCreated(networkId)
  })
  return (
    <GlassDialog
      open={open}
      onOpenChange={onOpenChange}
      title={m.network_create()}
      busy={create.pending}
      footer={
        <>
          <Button
            variant="ghost"
            className="h-9 rounded-full px-4"
            disabled={create.pending}
            onClick={() => onOpenChange(false)}
          >
            {m.dialog_cancel()}
          </Button>
          <Button
            type="submit"
            form="network-create-form"
            className="h-9 rounded-full px-4"
            disabled={create.pending}
            aria-busy={create.pending}
          >
            <BusyLabel busy={create.pending}>
              {m.network_create_submit()}
            </BusyLabel>
          </Button>
        </>
      }
    >
      <form
        id="network-create-form"
        noValidate
        className="pt-1"
        onSubmit={(event) => void create.submit(event)}
      >
        <NetworkNameField id="network-create-name" size="default" autoFocus />
      </form>
    </GlassDialog>
  )
}
