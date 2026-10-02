import { InfoIcon, TriangleAlertIcon } from "lucide-react"
import type { Metadata } from "next"
import { redirect } from "next/navigation"

import { AccountLoginForm } from "@/components/auth/account-login-form"
import { DiscardRejectedSession } from "@/components/auth/discard-rejected-session"
import { AnnounceSignedOut } from "@/components/auth/session-sync"
import { LoginForm } from "@/components/auth/login-form"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Separator } from "@/components/ui/separator"
import { controllerUrl } from "@/lib/controller"
import { safeLoginReturn } from "@/lib/device-login"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "登录", referrer: "no-referrer" }

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { reason, returnTo } = await searchParams
  const destination = safeLoginReturn(returnTo)
  const expired = reason === "expired"
  const unconfirmedLogout = reason === "logout-unconfirmed"
  const session = await readSession()
  // Only a plain visit with a live cookie skips the form. After a rejection
  // the cookie may still exist until it is discarded, so never bounce back.
  if (!expired && session.kind === "active") redirect(destination)
  const forDevice = destination.startsWith("/authorize-device?")

  return (
    <StandaloneFrame
      footer={
        <span>
          Controller <code className="font-mono break-all text-foreground/80">{controllerUrl()}</code>
        </span>
      }
    >
      <AnnounceSignedOut />
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">{forDevice ? "登录以批准设备" : "登录 Meshway"}</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          {forDevice
            ? "使用你的 Meshway 账户登录，然后确认 App 的接入请求。App 不会得到你的密码或登录会话。"
            : "使用邀请注册的邮箱和密码登录。会话保存在仅服务端可读的 Cookie 中，到期时间由 controller 决定。"}
        </p>
      </div>
      {expired ? (
        <>
          {session.kind !== "none" ? <DiscardRejectedSession /> : null}
          <Alert>
            <InfoIcon />
            <AlertTitle>需要重新登录</AlertTitle>
            <AlertDescription>会话已过期、已退出或已被撤销，controller 不再接受之前的登录。</AlertDescription>
          </Alert>
        </>
      ) : null}
      {unconfirmedLogout ? (
        <Alert variant="destructive">
          <TriangleAlertIcon />
          <AlertTitle>controller 未确认撤销</AlertTitle>
          <AlertDescription>
            本浏览器已退出并清除了本地会话与未保存的草稿，但 controller 当时不可用，未确认已撤销该会话；它可能在原到期时间前仍然有效。controller 恢复后可重新登录再退出，或撤销该账户的全部会话。
          </AlertDescription>
        </Alert>
      ) : null}
      <AccountLoginForm returnTo={destination} />
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        <Separator className="flex-1" />
        实例管理员
        <Separator className="flex-1" />
      </div>
      <LoginForm returnTo={destination} />
      <DiagnosticDetails title="账户与管理员令牌的区别">
        <span>
          账户由网络所有者通过邀请链接创建，登录后只能看到自己加入的网络，可在浏览器中批准自己的 App 设备。没有公开注册。
        </span>
        <span>
          管理员令牌是 controller 实例的兼容管理入口，不代表任何用户，不能批准用户设备。首次使用时，用它为网络认领第一个所有者账户。令牌在运行 controller 的机器上读取：
        </span>
        <pre className="overflow-x-auto rounded-md border bg-muted/50 px-3 py-2 font-mono text-[0.6875rem] text-foreground">
          cargo run -p meshway-controller -- admin-token --data-dir .meshway/controller
        </pre>
      </DiagnosticDetails>
    </StandaloneFrame>
  )
}
