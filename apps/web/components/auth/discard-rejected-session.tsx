"use client"

import { useEffect } from "react"

import { discardRejectedSession } from "@/lib/auth-actions"

/**
 * Cookies cannot be changed while a page renders, so the login page asks the
 * server (via a POST Server Action, i.e. a fresh request with the cookie the
 * browser holds now) to drop the stale session cookie. The action re-verifies
 * with the controller and leaves a valid session alone; then the rejection
 * belonged to an older sign-in, and the browser continues to `destination`.
 */
export function DiscardRejectedSession({ destination }: { destination: string }) {
  useEffect(() => {
    void discardRejectedSession().then((outcome) => {
      if (outcome === "kept") window.location.replace(destination)
    }, () => undefined)
  }, [destination])
  return null
}
