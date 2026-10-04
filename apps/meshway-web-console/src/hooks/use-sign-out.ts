import { useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useCallback } from "react"

import { signOut } from "@/api/sign-in"

/** Revokes the session, drops every cached read and returns to the login page. */
export function useSignOut(): () => Promise<void> {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  return useCallback(async () => {
    const outcome = await signOut()
    queryClient.clear()
    await navigate({
      to: "/login",
      search: {
        reason: outcome === "revoked" ? "signed_out" : "signed_out_unconfirmed",
      },
    })
  }, [navigate, queryClient])
}
