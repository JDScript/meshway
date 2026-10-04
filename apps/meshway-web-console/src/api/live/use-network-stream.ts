import { useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"

import { runNetworkStream, stopNetworkStream } from "./stream"

/**
 * This tab's one event stream, for the network it has open: from mount
 * until the network changes or the page unmounts (signing out leaves it).
 */
export function useNetworkStream(networkId: string) {
  const queryClient = useQueryClient()
  useEffect(() => {
    const controller = new AbortController()
    void runNetworkStream(queryClient, networkId, controller.signal)
    return () => {
      controller.abort()
      stopNetworkStream(queryClient, networkId)
    }
  }, [queryClient, networkId])
}
