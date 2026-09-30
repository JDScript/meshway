import { ArrowRightIcon, CloudOffIcon, ServerIcon } from "lucide-react"
import Link from "next/link"

import { RefreshButton } from "@/components/common/refresh-button"
import { buttonVariants } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"

export function ControllerUnavailable({ url, error }: { url: string; error: string }) {
  return (
    <Empty className="border border-dashed border-destructive/30 bg-destructive/[0.03]">
      <EmptyHeader>
        <EmptyMedia variant="icon" className="bg-destructive/10 text-destructive">
          <CloudOffIcon />
        </EmptyMedia>
        <EmptyTitle>无法读取 controller</EmptyTitle>
        <EmptyDescription>
          请求 <code className="font-mono">{url}</code> 失败。这不是登录失效；确认 controller
          正在运行，或检查服务端的 <code className="font-mono">MESHWAY_CONTROLLER_URL</code>。
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <pre className="max-w-full overflow-x-auto rounded-md border bg-background px-3 py-2 text-left font-mono text-[0.6875rem] whitespace-pre-wrap text-muted-foreground">
          {error}
        </pre>
        <RefreshButton />
      </EmptyContent>
    </Empty>
  )
}

export function NoNodes({ compact = false }: { compact?: boolean }) {
  return (
    <Empty className={compact ? "py-8" : "border border-dashed"}>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <ServerIcon />
        </EmptyMedia>
        <EmptyTitle>还没有节点</EmptyTitle>
        <EmptyDescription>
          创建一次性注册令牌，在节点上运行 agent 的 enroll 命令，注册完成后节点会出现在这里。
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Link href="/enrollment" className={buttonVariants({ size: "lg" })}>
          创建注册令牌
          <ArrowRightIcon data-icon="inline-end" />
        </Link>
      </EmptyContent>
    </Empty>
  )
}
