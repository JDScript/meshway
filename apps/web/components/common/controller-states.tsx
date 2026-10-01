import { ArrowRightIcon, CloudOffIcon, ServerIcon } from "lucide-react"
import Link from "next/link"

import { DiagnosticDetails } from "@/components/common/diagnostic-details"
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

/** A read failed: say so, offer a retry, keep the request details folded. */
export function ControllerUnavailable({ url, error }: { url: string; error: string }) {
  return (
    <Empty className="rounded-lg border border-dashed border-destructive/30 bg-destructive/[0.03] py-12">
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
        <RefreshButton />
        <DiagnosticDetails title="请求技术详情" className="w-full bg-card text-left">
          <p className="font-mono break-all">{url}</p>
          <pre className="max-w-full font-mono whitespace-pre-wrap [overflow-wrap:anywhere]">{error}</pre>
          <p>Web 服务端配置：MESHWAY_CONTROLLER_URL</p>
        </DiagnosticDetails>
      </EmptyContent>
    </Empty>
  )
}

/** The network really has no members yet. */
export function NoNodes({ addHref, compact = false }: { addHref: string; compact?: boolean }) {
  return (
    <Empty className={compact ? "py-10" : "rounded-lg border border-dashed bg-card py-14"}>
      <EmptyHeader>
        <EmptyMedia variant="icon" className="bg-brand/10 text-brand">
          <ServerIcon />
        </EmptyMedia>
        <EmptyTitle>这张网络还没有节点</EmptyTitle>
        <EmptyDescription>
          生成一次性接入令牌，在服务器上运行 agent 的 enroll 命令；Meshway App 通过浏览器授权接入。注册完成后节点会出现在这里。
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Link href={addHref} className={buttonVariants({ size: "lg" })}>
          添加节点
          <ArrowRightIcon data-icon="inline-end" />
        </Link>
      </EmptyContent>
    </Empty>
  )
}
