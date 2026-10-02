"use client"

import { useEffect } from "react"

import { useDraftVault } from "@/components/network/network-workspace"
import { currentSessionKey } from "@/lib/auth-actions"

const CHANNEL = "meshway-console-session"

/** Tells other open console tabs that the sign-in on this browser changed. */
export function useAnnounceSession(key: string) {
  useEffect(() => {
    if (typeof BroadcastChannel === "undefined") return
    const channel = new BroadcastChannel(CHANNEL)
    channel.postMessage(key)
    channel.close()
  }, [key])
}

/** Login page: any open console must stop treating its drafts as current. */
export function AnnounceSignedOut() {
  useAnnounceSession("signed-out")
  return null
}

/**
 * Inside the console: the page was rendered for sign-in `sessionKey`. When
 * another tab signs in or out, or this tab regains focus with a different
 * cookie, every unsaved draft (and its CAS base) of the old sign-in is
 * dropped and the page reloads under the current one, so account A's work
 * can never become the base of an action sent with account B's session.
 */
export function SessionSync({ sessionKey }: { sessionKey: string }) {
  const vault = useDraftVault()
  useAnnounceSession(sessionKey)
  useEffect(() => {
    let stopped = false
    const changed = () => {
      if (stopped) return
      stopped = true
      vault.reset()
      window.location.reload()
    }
    const check = () => {
      if (document.visibilityState !== "visible" || stopped) return
      currentSessionKey().then((current) => {
        if (current !== sessionKey) changed()
      }, () => undefined)
    }
    const channel = typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel(CHANNEL)
    if (channel) channel.onmessage = (event: MessageEvent<unknown>) => {
      if (event.data !== sessionKey) changed()
    }
    window.addEventListener("focus", check)
    document.addEventListener("visibilitychange", check)
    return () => {
      channel?.close()
      window.removeEventListener("focus", check)
      document.removeEventListener("visibilitychange", check)
    }
  }, [sessionKey, vault])
  return null
}
