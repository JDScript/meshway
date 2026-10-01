import { InfoIcon } from "lucide-react"
import type { Metadata } from "next"
import { redirect } from "next/navigation"

import { DiscardRejectedSession } from "@/components/auth/discard-rejected-session"
import { LoginForm } from "@/components/auth/login-form"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { controllerUrl } from "@/lib/controller"
import { safeLoginReturn } from "@/lib/device-login"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "登录" }

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { reason, returnTo } = await searchParams
  const destination = safeLoginReturn(returnTo)
  const expired = reason === "expired"
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
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">登录控制台</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          {forDevice ? "登录后继续批准 Meshway App 的接入请求。" : "使用 controller 的管理员令牌登录。"}
          令牌先向 controller 验证，再保存在仅服务端可读的 Cookie 中，8 小时后需重新登录。
        </p>
      </div>
      {expired ? (
        <>
          {session.kind !== "none" ? <DiscardRejectedSession /> : null}
          <Alert>
            <InfoIcon />
            <AlertTitle>需要重新登录</AlertTitle>
            <AlertDescription>会话已过期，或 controller 不再接受之前的管理员令牌。</AlertDescription>
          </Alert>
        </>
      ) : null}
      <LoginForm returnTo={destination} />
      <DiagnosticDetails title="在哪里找到管理员令牌">
        <span>在运行 controller 的机器上读取：</span>
        <pre className="overflow-x-auto rounded-md border bg-muted/50 px-3 py-2 font-mono text-[0.6875rem] text-foreground">
          cargo run -p meshway-controller -- admin-token --data-dir .meshway/controller
        </pre>
        <span>用户账户（邮箱 + 密码、邀请加入）上线后，日常登录将不再需要管理员令牌。</span>
      </DiagnosticDetails>
    </StandaloneFrame>
  )
}
