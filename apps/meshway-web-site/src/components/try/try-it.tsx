"use client"

import { Dialog } from "@base-ui/react/dialog"
import { Button } from "@meshway/ui/button"
import type { APIPlaygroundProps } from "fumadocs-openapi"
import PlaygroundClient, {
  DefaultResultDisplay,
  type ResultDisplayProps,
} from "fumadocs-openapi/ui/playground/client"
import { useI18n } from "fumadocs-ui/contexts/i18n"
import { useEffect, useRef, useState } from "react"

import { i18n, type Lang } from "@/lib/i18n"
import {
  dropTrySession,
  signOutOfTry,
  startSignIn,
  trySession,
  useTrySession,
} from "@/lib/try-it"

const words = {
  zh: {
    signedOut:
      "在控制台授权本站后，可以在这里以你的身份发送请求；授权 1 小时后失效，只保存在这个页面里。",
    signIn: "在控制台授权",
    signedIn: (email: string, minutes: number) =>
      `以 ${email} 的身份试用 · ${minutes} 分钟后失效`,
    signOut: "结束试用",
    notSignedIn: "请先在控制台授权本站。",
    cancelled: "已取消，没有发送。",
    writeTitle: "发送这个写请求？",
    writeText: (method: string, path: string) =>
      `${method} ${path} 会以你的身份在服务端执行，真实生效。`,
    send: "发送",
    keep: "取消",
  },
  en: {
    signedOut:
      "Authorize this site in the console to send requests here as you; the authorization lasts one hour and is kept on this page only.",
    signIn: "Authorize in the console",
    signedIn: (email: string, minutes: number) =>
      `Trying as ${email} · expires in ${minutes} min`,
    signOut: "Stop trying",
    notSignedIn: "Authorize this site in the console first.",
    cancelled: "Cancelled; nothing was sent.",
    writeTitle: "Send this write request?",
    writeText: (method: string, path: string) =>
      `${method} ${path} runs on the server as you, for real.`,
    send: "Send",
    keep: "Cancel",
  },
}

const langOf = (locale: string | undefined): Lang =>
  i18n.languages.find((lang) => lang === locale) ?? i18n.defaultLanguage

/** A reply that says the session is no longer taken drops it, so the page asks again. */
function ResultDisplay(props: ResultDisplayProps) {
  const rejected = props.data.type === "response" && props.data.status === 401
  useEffect(() => {
    if (rejected) dropTrySession()
  }, [rejected])
  return <DefaultResultDisplay data-meshway-result {...props} />
}

/**
 * The playground of one operation, sending with the delegated session
 * (never a field of the form: Fumadocs keeps those in localStorage). A
 * write is confirmed before it goes.
 */
export function TryIt({ method, path }: APIPlaygroundProps) {
  const { locale } = useI18n()
  const lang = langOf(locale)
  const text = words[lang]
  const session = useTrySession()
  const [now, setNow] = useState(() => Date.now())
  const [asking, setAsking] = useState(false)
  const answer = useRef<((send: boolean) => void) | null>(null)

  useEffect(() => {
    if (!session) return
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [session])

  const decide = (send: boolean) => {
    answer.current?.(send)
    answer.current = null
    setAsking(false)
  }

  const onRequestInit = async (init: RequestInit): Promise<RequestInit> => {
    const current = trySession()
    if (!current) throw new Error(text.notSignedIn)
    const verb = (init.method ?? "GET").toUpperCase()
    if (verb !== "GET" && verb !== "HEAD") {
      const send = await new Promise<boolean>((resolve) => {
        answer.current = resolve
        setAsking(true)
      })
      if (!send) throw new Error(text.cancelled)
    }
    const headers = new Headers(init.headers)
    headers.set("Authorization", `Bearer ${current.token}`)
    return { ...init, headers }
  }

  const live = session && session.expiresAt > now ? session : null
  const minutes = live
    ? Math.max(1, Math.ceil((live.expiresAt - now) / 60_000))
    : 0

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 rounded-xl border bg-fd-card px-3.5 py-2.5 text-sm">
        <p className="min-w-0 flex-[1_1_16rem] text-fd-muted-foreground">
          {live ? text.signedIn(live.email, minutes) : text.signedOut}
        </p>
        {live ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void signOutOfTry()}
          >
            {text.signOut}
          </Button>
        ) : (
          <Button
            type="button"
            size="sm"
            onClick={() => void startSignIn(lang)}
          >
            {text.signIn}
          </Button>
        )}
      </div>
      <PlaygroundClient
        transformAuthInputs={() => []}
        fetchOptions={{ onRequestInit }}
        components={{ ResultDisplay }}
      />
      <Dialog.Root
        open={asking}
        onOpenChange={(open) => {
          if (!open) decide(false)
        }}
      >
        <Dialog.Portal>
          <Dialog.Backdrop className="fixed inset-0 z-50 bg-black/30 backdrop-blur-[2px]" />
          <Dialog.Popup className="glass fixed top-1/2 left-1/2 z-50 flex w-[min(92vw,26rem)] -translate-1/2 flex-col gap-3 rounded-surface p-5 text-fd-foreground">
            <Dialog.Title className="text-lead font-semibold">
              {text.writeTitle}
            </Dialog.Title>
            <Dialog.Description className="text-sm text-fd-muted-foreground">
              {text.writeText(method.toUpperCase(), path)}
            </Dialog.Description>
            <div className="mt-2 flex justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={() => decide(false)}
              >
                {text.keep}
              </Button>
              <Button type="button" onClick={() => decide(true)}>
                {text.send}
              </Button>
            </div>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  )
}
