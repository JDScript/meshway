import { useEffect } from "react"

import { rejectSession } from "@/lib/api"
import { currentSession } from "@/lib/session"

/** setTimeout's longest delay; a session can outlive it (up to 30 days). */
const MAX_DELAY_MS = 2 ** 31 - 1

/**
 * Signs this tab out when its session reaches the controller's absolute
 * expiry, so a page left open does not keep showing a sign-in that has
 * ended. A revocation elsewhere still shows on the next request.
 */
export function useSessionExpiry() {
  useEffect(() => {
    const session = currentSession()
    if (!session) return
    let timer: number | undefined
    const wait = () => {
      const left = session.expiresAt - Date.now()
      if (left <= 0) return rejectSession(session)
      timer = window.setTimeout(wait, Math.min(left, MAX_DELAY_MS))
    }
    wait()
    return () => window.clearTimeout(timer)
  }, [])
}
