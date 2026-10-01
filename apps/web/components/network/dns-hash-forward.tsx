"use client"

import { useRouter } from "next/navigation"
import { useEffect } from "react"

/** Fragments never reach the server, so `/network#dns-…` is forwarded here. */
export function DnsHashForward({ href }: { href: string }) {
  const router = useRouter()
  useEffect(() => {
    const hash = window.location.hash
    if (hash.startsWith("#dns")) router.replace(`${href}${hash}`)
  }, [href, router])
  return null
}
