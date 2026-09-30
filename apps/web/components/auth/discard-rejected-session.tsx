"use client"

import { useEffect } from "react"

import { discardRejectedSession } from "@/lib/auth-actions"

/**
 * Cookies cannot be changed while a page renders, so the login page asks the
 * server (via a POST Server Action) to drop the stale session cookie. The
 * action re-verifies with the controller and leaves a valid session alone.
 */
export function DiscardRejectedSession() {
  useEffect(() => {
    void discardRejectedSession()
  }, [])
  return null
}
