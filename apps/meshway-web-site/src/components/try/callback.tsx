"use client"

import { buttonVariants } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { useEffect, useRef, useState } from "react"

import { localized, type Lang } from "@/lib/i18n"
import { finishSignIn, type CallbackOutcome } from "@/lib/try-it"

const words = {
  zh: {
    working: "正在完成授权…",
    state:
      "这次授权不是从这个页面开始的，或已经处理过。请回到 API 参考重新开始。",
    exchange: "授权已失效或已被使用。请回到 API 参考重新开始。",
    network: "连接不到服务端，授权没有完成。请回到 API 参考重新开始。",
    back: "返回 API 参考",
  },
  en: {
    working: "Finishing the authorization…",
    state:
      "This authorization did not start on this page, or it was already handled. Go back to the API reference and start again.",
    exchange:
      "The authorization has expired or was already used. Go back to the API reference and start again.",
    network:
      "The server could not be reached, so the authorization did not finish. Go back to the API reference and start again.",
    back: "Back to the API reference",
  },
} satisfies Record<Lang, Record<string, string>>

/** Where the console returns with a code: exchanged here, then back to the page that started. */
export function SignInCallback({ lang }: { lang: Lang }) {
  const router = useRouter()
  const [outcome, setOutcome] = useState<CallbackOutcome | null>(null)
  const started = useRef(false)

  useEffect(() => {
    // The code is good once: a second effect run must not exchange it again.
    if (started.current) return
    started.current = true
    void finishSignIn().then((result) => {
      if (result.kind === "signed-in") router.replace(result.returnTo)
      else setOutcome(result)
    })
  }, [router])

  const text = words[lang]
  return (
    <main className="flex flex-1 items-center justify-center p-8">
      <div className="flex max-w-md flex-col items-center gap-5 text-center">
        <p className="text-lead text-muted-foreground" role="status">
          {outcome?.kind === "failed" ? text[outcome.reason] : text.working}
        </p>
        {outcome?.kind === "failed" ? (
          <Link
            href={localized(lang, "/docs/api")}
            className={cn(
              buttonVariants({ size: "lg" }),
              "h-10 rounded-tile px-4"
            )}
          >
            {text.back}
          </Link>
        ) : null}
      </div>
    </main>
  )
}
