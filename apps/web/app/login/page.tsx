import { InfoIcon } from "lucide-react"
import type { Metadata } from "next"
import { redirect } from "next/navigation"

import { DiscardRejectedSession } from "@/components/auth/discard-rejected-session"
import { LoginForm } from "@/components/auth/login-form"
import { BrandMark } from "@/components/shell/brand"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { controllerUrl } from "@/lib/controller"
import { readSession } from "@/lib/session"
import { safeLoginReturn } from "@/lib/device-login"

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

  return (
    <main className="grid min-h-svh lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
      <section className="relative hidden flex-col justify-between overflow-hidden border-r bg-canvas p-10 lg:flex">
        <div aria-hidden="true" className="bg-grid-dots absolute inset-0 opacity-60 [mask-image:radial-gradient(ellipse_at_center,black_30%,transparent_75%)]" />
        <div className="relative flex items-center gap-2.5">
          <BrandMark />
          <span className="text-base font-semibold tracking-tight">Meshway</span>
        </div>
        <div className="relative flex max-w-md flex-col gap-3">
          <h2 className="text-2xl font-semibold tracking-tight">
            可编程的 mesh 网络控制台
          </h2>
          <p className="text-sm text-muted-foreground">
            管理节点身份与 Mesh 地址，用有序规则为域名、CIDR 和国家规划经由节点与出口。
          </p>
        </div>
        <p className="relative font-mono text-xs text-muted-foreground">{controllerUrl()}</p>
      </section>

      <section className="flex items-center justify-center px-4 py-12">
        <div className="flex w-full max-w-sm flex-col gap-6">
          <div className="flex items-center gap-2.5 lg:hidden">
            <BrandMark />
            <span className="text-base font-semibold tracking-tight">Meshway</span>
          </div>
          <div className="flex flex-col gap-1.5">
            <h1 className="text-xl font-semibold tracking-tight">登录控制台</h1>
            <p className="text-sm text-muted-foreground">
              输入 controller 的管理员令牌。令牌先向 controller 验证，再保存在仅服务端可读的 Cookie 中，8 小时后需重新登录。
            </p>
          </div>
          {expired ? (
            <>
              {session.kind !== "none" ? <DiscardRejectedSession /> : null}
              <Alert>
                <InfoIcon />
                <AlertTitle>需要重新登录</AlertTitle>
                <AlertDescription>
                  会话已过期，或 controller 不再接受之前的管理员令牌。
                </AlertDescription>
              </Alert>
            </>
          ) : null}
          <LoginForm returnTo={destination} />
          <div className="flex flex-col gap-2 border-t pt-5 text-xs text-muted-foreground">
            <span>在运行 controller 的机器上读取令牌：</span>
            <pre className="overflow-x-auto rounded-lg border bg-muted/40 px-3 py-2 font-mono text-[0.6875rem] text-foreground">
              cargo run -p meshway-controller -- admin-token --data-dir .meshway/controller
            </pre>
            <span className="lg:hidden">
              Controller <code className="font-mono break-all">{controllerUrl()}</code>
            </span>
          </div>
        </div>
      </section>
    </main>
  )
}
