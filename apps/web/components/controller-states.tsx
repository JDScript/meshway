import { ArrowRightIcon, CircleAlertIcon, InfoIcon } from "lucide-react"
import Link from "next/link"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { buttonVariants } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"

export function ControllerUnavailable({
  url,
  error,
}: {
  url: string
  error: string
}) {
  return (
    <Alert variant="destructive" className="px-4 py-3 text-sm">
      <CircleAlertIcon />
      <AlertTitle className="text-sm">无法读取 controller</AlertTitle>
      <AlertDescription className="flex flex-col gap-2 text-sm">
        <p>
          请求 <code className="font-mono text-xs">{url}</code> 失败。确认
          controller 已启动并监听该地址，或通过{" "}
          <code className="font-mono text-xs">MESHWAY_CONTROLLER_URL</code>{" "}
          指向正确的地址。这不是登录失效；controller 恢复后重新读取即可。
        </p>
        <pre className="overflow-x-auto rounded-md bg-destructive/5 px-3 py-2 font-mono text-xs whitespace-pre-wrap">
          {error}
        </pre>
      </AlertDescription>
    </Alert>
  )
}

export function NoNodes() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">还没有注册节点</CardTitle>
        <CardDescription className="text-sm">
          Controller 可以访问，但还没有 agent 完成注册。先创建一次性注册令牌，再在节点上用它运行
          agent 的 enroll 命令；注册成功后节点会出现在这里。
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Link
          href="/enrollment"
          className={buttonVariants({ variant: "outline", size: "lg" })}
        >
          创建注册令牌
          <ArrowRightIcon data-icon="inline-end" />
        </Link>
      </CardContent>
    </Card>
  )
}

export function DevelopmentNotice() {
  return (
    <Alert className="px-4 py-3 text-sm">
      <InfoIcon />
      <AlertTitle className="text-sm">开发阶段：仅有控制面注册</AlertTitle>
      <AlertDescription className="text-sm">
        节点用一次性注册令牌向 controller 注册，并持有自己的控制面凭据；心跳会更新「最后联系」，撤销后
        controller 拒绝该凭据。这是基于令牌的控制面身份，不是数据面密钥证明。目前没有 TUN
        数据面、隧道、路由策略、DNS 或端到端加密，注册记录不代表节点在线或 mesh 已连通。
      </AlertDescription>
    </Alert>
  )
}
