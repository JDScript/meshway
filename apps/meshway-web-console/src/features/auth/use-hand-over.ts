import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useCallback } from "react"

import { currentSession, endSession } from "@/state/session"

import type { LoginReason } from "./login-reasons"

/**
 * After a change the controller made that ended this tab's session
 * (changing the password, revoking every sign-in, a reset): forgets the
 * session here, drops every cached read and opens the sign-in, which says
 * why.
 */
export function useHandOver(): (reason: LoginReason) => Promise<void> {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  return useCallback(
    async (reason) => {
      const session = currentSession()
      if (session) endSession(session)
      await queryClient.cancelQueries()
      queryClient.clear()
      await navigate({ to: "/login", search: { reason } })
    },
    [navigate, queryClient]
  )
}
