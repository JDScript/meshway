import {
  renameNetwork,
  type NetworkSummary,
  type RenameNetworkRequest,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useRef, useState } from "react"
import { toast } from "sonner"

import { ApiFailure, authed, client, problemCode } from "@/api/client"
import { failureText } from "@/api/failure"
import { networkQuery, networksQuery } from "@/api/queries/network"
import { NAME_FAILURE } from "@/components/network/create-network"
import { useNetworkNameCheck } from "@/hooks/use-network-name-check"
import { m } from "@/paraglide/messages.js"
import { attempts } from "@/state/attempts"

const TOAST = "network-rename"

type RenameBody = Omit<RenameNetworkRequest, "requestId">

/** By network. */
const renames = attempts<RenameBody>()

/**
 * Renaming the network: entity metadata, saved on its own (not part of the
 * configuration, whose revision it leaves alone). `expectedName` is the
 * name last read from the controller, so a rename made meanwhile by
 * someone else is not overwritten; after an attempt whose outcome is
 * unknown, the same name is the same request again.
 */
export function useRename(network: NetworkSummary) {
  const queryClient = useQueryClient()
  const checkName = useNetworkNameCheck()
  const sending = useRef(false)
  const [pending, setPending] = useState(false)

  const rename = async (typed: string): Promise<boolean> => {
    if (sending.current) return false
    const checked = checkName(typed)
    if ("refused" in checked) {
      toast.error(checked.refused, { id: TOAST })
      return false
    }
    const { name } = checked
    sending.current = true
    setPending(true)
    try {
      const renamed = await renames.send(
        network.id,
        name,
        () => ({ name, expectedName: network.name }),
        (attempt) =>
          authed((token) =>
            renameNetwork(
              client,
              { networkId: network.id },
              { requestId: attempt.requestId, ...attempt.body },
              { token }
            )
          )
      )
      queryClient.setQueryData(networkQuery(network.id).queryKey, renamed)
      void queryClient.invalidateQueries({
        queryKey: networksQuery().queryKey,
      })
      toast.success(m.settings_renamed(), { id: TOAST })
      return true
    } catch (error) {
      const failure = error instanceof ApiFailure ? error.failure : null
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
