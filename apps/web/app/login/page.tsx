import { InfoIcon } from "lucide-react"
import type { Metadata } from "next"
import { redirect } from "next/navigation"

import { Brand } from "@/components/brand"
import { DiscardRejectedSession } from "@/components/discard-rejected-session"
import { LoginForm } from "@/components/login-form"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { controllerUrl } from "@/lib/controller"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "登录" }

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { reason } = await searchParams
  const expired = reason === "expired"
  const session = await readSession()
  // Only a plain visit with a live cookie skips the form. After a rejection
  // the cookie may still exist until it is discarded, so never bounce back.
  if (!expired && session.kind === "active") redirect("/")

  return (
    <main className="flex min-h-svh items-center justify-center px-4 py-12">
      <div className="flex w-full max-w-sm flex-col gap-6">
        <Brand />
        {expired ? (
          <>
            {session.kind !== "none" ? <DiscardRejectedSession /> : null}
            <Alert className="px-4 py-3 text-sm">
              <InfoIcon />
              <AlertTitle className="text-sm">需要重新登录</AlertTitle>
              <AlertDescription className="text-sm">
                会话已过期，或 controller 不再接受之前的管理员令牌。
              </AlertDescription>
            </Alert>
          </>
        ) : null}
        <Card className="[--card-spacing:--spacing(5)]">
          <CardHeader>
            <CardTitle className="text-base">管理员登录</CardTitle>
            <CardDescription className="text-sm">
              输入 controller 的管理员令牌。令牌会先向 controller 验证，再保存在仅服务端可读的 Cookie
              中，8 小时后需要重新登录。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <LoginForm />
          </CardContent>
          <CardFooter className="flex flex-col items-start gap-2 text-sm text-muted-foreground">
            <p>在运行 controller 的机器上读取令牌：</p>
            <pre className="w-full overflow-x-auto rounded-md border bg-muted/50 px-3 py-2 font-mono text-xs">
              cargo run -p meshway-controller -- admin-token --data-dir .meshway/controller
            </pre>
            <p>
              Controller 地址{" "}
              <code className="font-mono text-xs break-all">{controllerUrl()}</code>
              ，由服务端环境变量{" "}
              <code className="font-mono text-xs">MESHWAY_CONTROLLER_URL</code> 配置。
            </p>
          </CardFooter>
        </Card>
      </div>
    </main>
  )
}
