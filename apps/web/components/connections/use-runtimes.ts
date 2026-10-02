"use client"

import { useActionScope } from "@/components/network/network-workspace"

import type { ConnectionRuntimesSnapshot } from "@meshway/control-client"
import { useCallback, useEffect, useRef, useState, useTransition } from "react"

import type { RuntimeReadResult } from "@/lib/action-state"
import { readConnectionRuntimesAction } from "@/lib/connections-actions"

/** Bounded background refresh while the page is visible. */
const REFRESH_MS = 12_000
/** Reports keep ageing on screen between reads. */
const TICK_MS = 5_000

export type InitialRuntimes =
  | { ok: true; data: ConnectionRuntimesSnapshot; fetchedAt: number }
  | { ok: false; error: string; fetchedAt: number }

export type Runtimes = {
  /** Last successful read, kept on later errors (and shown as such). */
  snapshot: ConnectionRuntimesSnapshot | null
  fetchedAt: number | null
  error: string | null
  /** Clock used to age reports; advances while the page is open. */
  now: number
  refreshing: boolean
  refresh: () => void
}

/**
 * Live connection observations, read separately from the connection plan.
 * Refreshing never touches any editor state: drafts keep their own base.
 * `read` lets a page fold other status reads into the same single poll
 * (one interval, one request per tick) and hand back the runtime part.
 */
export function useConnectionRuntimes(
  initial: InitialRuntimes,
  read?: () => Promise<RuntimeReadResult>
): Runtimes {
  const actionScope = useActionScope()
  const scopedRead = useCallback(() => readConnectionRuntimesAction(actionScope), [actionScope])
  const chosenRead = read ?? scopedRead
  const reader = useRef(chosenRead)
  useEffect(() => {
    reader.current = chosenRead
  }, [chosenRead])
  const [snapshot, setSnapshot] = useState(initial.ok ? initial.data : null)
  const [fetchedAt, setFetchedAt] = useState(initial.ok ? initial.fetchedAt : null)
  const [error, setError] = useState(initial.ok ? null : initial.error)
  const [now, setNow] = useState(initial.fetchedAt)
  const [refreshing, startRefresh] = useTransition()
  const sequence = useRef(0)

  const refresh = useCallback(() => {
    const seq = ++sequence.current
    startRefresh(async () => {
      const result = await reader.current()
      // A newer read (or unmount) supersedes this one.
      if (seq !== sequence.current) return
      setNow(Date.now())
      if (result.status === "loaded") {
        setSnapshot(result.snapshot)
        setFetchedAt(result.fetchedAt)
        setError(null)
      } else {
        setError(result.message)
      }
    })
  }, [])

  useEffect(() => {
    let timer: number | undefined
    const schedule = () => {
      window.clearInterval(timer)
      if (document.visibilityState === "visible") timer = window.setInterval(refresh, REFRESH_MS)
    }
    const onVisibility = () => {
      if (document.visibilityState === "visible") refresh()
      schedule()
    }
    schedule()
    document.addEventListener("visibilitychange", onVisibility)
    const tick = window.setInterval(() => setNow(Date.now()), TICK_MS)
    const owned = sequence
    return () => {
      owned.current += 1
      window.clearInterval(timer)
      window.clearInterval(tick)
      document.removeEventListener("visibilitychange", onVisibility)
    }
  }, [refresh])

  return { snapshot, fetchedAt, error, now, refreshing, refresh }
}
