import { CheckCircle2Icon, InfoIcon, TriangleAlertIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"
import { redirect } from "next/navigation"
import type { ReactNode } from "react"

import { AccountLoginForm } from "@/components/auth/account-login-form"
import { DiscardRejectedSession } from "@/components/auth/discard-rejected-session"
import { AnnounceSignedOut } from "@/components/auth/session-sync"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { controllerUrl } from "@/lib/controller"
import { safeLoginReturn } from "@/lib/device-login"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "登录", referrer: "no-referrer" }

const notices: Record<string, { tone: "info" | "success" | "destructive"; title: string; body: ReactNode }> = {
  expired: { tone: "info", title: "需要重新登录", body: "会话已过期、已退出或已被撤销。" },
  "logout-unconfirmed": {
    tone: "destructive",
    title: "controller 未确认撤销",
    body: "本浏览器已退出并清除了草稿，但 controller 当时不可用，之前的会话在原到期时间前可能仍然有效。恢复后可重新登录，在「我的账户」中退出所有会话。",
  },
  "password-changed": { tone: "success", title: "密码已修改", body: "所有旧会话已失效，请用新密码登录。" },
  "password-reset": { tone: "success", title: "密码已重置", body: "所有旧会话已失效，请用新密码登录。" },
  "sessions-revoked": { tone: "success", title: "已退出所有会话", body: "这个账户在所有浏览器上的登录都已撤销。" },
}

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { reason, returnTo } = await searchParams
  const destination = safeLoginReturn(returnTo)
  const expired = reason === "expired"
  const notice = typeof reason === "string" ? notices[reason] : undefined
  const session = await readSession()
  // Only a plain visit with a live cookie skips the form. After a rejection
  // the cookie may still exist until it is discarded, so never bounce back;
  // an outcome notice (password changed / reset, signed out everywhere) is
  // always shown, even if the browser holds another sign-in.
  if (!expired && notice === undefined && session.kind === "active") redirect(destination)
  const forDevice = destination.startsWith("/authorize-device?")

  return (
    <StandaloneFrame
      footer={
        <>
          <span>没有账户？请向网络所有者索取邀请链接。</span>
          <span className="flex items-center justify-center gap-1.5">
            <span className="font-mono">{hostOf(controllerUrl())}</span>
            <span aria-hidden="true">·</span>
            <Link href="/setup" className="underline-offset-4 hover:text-foreground hover:underline">
              实例初始化与恢复
            </Link>
          </span>
        </>
      }
    >
      <AnnounceSignedOut />
      <div className="flex flex-col gap-1">
        <h1 className="text-xl font-semibold tracking-tight">{forDevice ? "登录以批准设备" : "登录"}</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          {forDevice ? "登录后确认 Meshway App 的接入请求。App 不会得到你的密码或登录会话。" : "使用你的 Meshway 账户。"}
        </p>
      </div>
      {expired && session.kind !== "none" ? <DiscardRejectedSession destination={destination} /> : null}
      {notice ? (
        <Alert variant={notice.tone === "destructive" ? "destructive" : "default"}>
          {notice.tone === "success" ? <CheckCircle2Icon /> : notice.tone === "destructive" ? <TriangleAlertIcon /> : <InfoIcon />}
          <AlertTitle>{notice.title}</AlertTitle>
          <AlertDescription>{notice.body}</AlertDescription>
        </Alert>
      ) : null}
      {!expired && notice !== undefined && session.kind === "active" ? (
        <p className="text-xs text-muted-foreground">
          这个浏览器目前仍有一个登录会话。
          <Link href={destination} className="ml-1 underline underline-offset-4 hover:text-foreground">
            继续使用它
          </Link>
          ，或在下方用其他账户登录。
        </p>
      ) : null}
      <AccountLoginForm returnTo={destination} />
    </StandaloneFrame>
  )
}

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}
