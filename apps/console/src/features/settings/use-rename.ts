import {
  renameNetwork,
  type NetworkSummary,
  type RenameNetworkRequest,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useRef, useState } from "react"
import { toast } from "sonner"

import { NAME_FAILURE } from "@/features/network/create-network"
import { networkQuery, networksQuery } from "@/features/network/queries"
import { ApiFailure, authed, client, problemCode } from "@/lib/api"
import { attemptFor, outcomeUnknown, type Attempt } from "@/lib/attempt"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

const TOAST = "network-rename"

type RenameBody = Omit<RenameNetworkRequest, "requestId">

/**
 * Renaming the network: entity metadata, saved on its own (not part of the
 * configuration, whose revision it leaves alone). `expectedName` is the
 * name last read from the controller, so a rename made meanwhile by
 * someone else is not overwritten; after an attempt whose outcome is
 * unknown, the same name is the same request again.
 */
export function useRename(network: NetworkSummary) {
  const queryClient = useQueryClient()
  const unsettled = useRef<Attempt<RenameBody> | null>(null)
  const sending = useRef(false)
  const [pending, setPending] = useState(false)

  const rename = async (typed: string): Promise<boolean> => {
    if (sending.current) return false
    const name = typed.trim()
    if (!name) {
      toast.error(m.network_name_missing(), { id: TOAST })
      return false
    }
    const attempt = attemptFor(unsettled.current, name, () => ({
      name,
      expectedName: network.name,
    }))
    sending.current = true
    setPending(true)
    try {
      const renamed = await authed((token) =>
        renameNetwork(
          client,
          { networkId: network.id },
          { requestId: attempt.requestId, ...attempt.body },
          { token }
        )
      )
      unsettled.current = null
      queryClient.setQueryData(networkQuery(network.id).queryKey, renamed)
      void queryClient.invalidateQueries({
        queryKey: networksQuery().queryKey,
      })
      toast.success(m.settings_renamed(), { id: TOAST })
      return true
    } catch (error) {
      const failure = error instanceof ApiFailure ? error.failure : null
      unsettled.current = failure && outcomeUnknown(failure) ? attempt : null
      const text = failureText(error, NAME_FAILURE)
      if (text) toast.error(text, { id: TOAST })
      // Renamed meanwhile, or no longer allowed: read the network again.
      const code = failure ? problemCode(failure) : null
      if (code === "name_conflict" || code === "permission_denied")
        void queryClient.invalidateQueries({
          queryKey: networkQuery(network.id).queryKey,
        })
      return false
    } finally {
      sending.current = false
      setPending(false)
    }
  }

  return { pending, rename }
}
