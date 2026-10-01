import { ArrowRightIcon, CloudOffIcon, ServerIcon } from "lucide-react"
import Link from "next/link"

import { RefreshButton } from "@/components/common/refresh-button"
import { DiagnosticDetails } from "@/components/common/diagnostic-details"
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
          请重试；持续失败时确认控制面正在运行，并检查 Web 服务端的控制面地址配置。
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <DiagnosticDetails title="请求技术详情" className="w-full text-left"><p className="font-mono break-all">{url}</p><pre className="max-w-full font-mono whitespace-pre-wrap [overflow-wrap:anywhere]">{error}</pre><p>Web 服务端配置：MESHWAY_CONTROLLER_URL</p></DiagnosticDetails>
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
