import { ArrowUpRightIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { docsUrl } from "@/lib/config"

type Status = "implemented" | "confirmed" | "design"

const statusBadge: Record<Status, { label: string; variant: "default" | "secondary" | "outline" }> = {
  implemented: { label: "已实现", variant: "default" },
  confirmed: { label: "已确认 · 未实现", variant: "secondary" },
  design: { label: "设计中", variant: "outline" },
}

const milestones: { title: string; detail: string; status: Status }[] = [
  {
    title: "节点上报与列表",
    detail: "agent 通过开发用 report 请求登记名称、版本和最后上报时间。",
    status: "implemented",
  },
  {
    title: "安全注册 · Enrollment",
    detail: "节点身份与准入凭据。当前的上报不做认证。",
    status: "design",
  },
  {
    title: "TUN 数据面与隧道",
    detail: "节点之间的实际数据通道。目前不存在任何隧道。",
    status: "design",
  },
  {
    title: "端到端加密",
    detail: "端到端加密优先；逐跳可信路由需显式选择，连接失败时不静默降级。",
    status: "design",
  },
  {
    title: "路由与必经节点",
    detail: "逻辑路径由策略决定，传输切换不得绕过必经节点。",
    status: "design",
  },
  {
    title: "Multi-transport",
    detail: "首版采用优先级主备；按流负载分担与同流多路径聚合不在首版范围。",
    status: "confirmed",
  },
  {
    title: "DNS",
    detail: "节点名称解析。",
    status: "design",
  },
]

export function Milestones() {
  return (
    <Card className="[--card-spacing:--spacing(5)]">
      <CardHeader>
        <CardTitle className="text-base">功能状态</CardTitle>
        <CardDescription className="text-sm">
          区分已实现功能与设计中的里程碑。只有标记为已实现的部分在当前版本中可用。
        </CardDescription>
        <CardAction>
          <a
            href={docsUrl}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            设计文档
            <ArrowUpRightIcon className="size-3.5" />
          </a>
        </CardAction>
      </CardHeader>
      <CardContent>
        <ul className="divide-y">
          {milestones.map(({ title, detail, status }) => (
            <li
              key={title}
              className="flex flex-col gap-1.5 py-3 first:pt-0 last:pb-0 sm:flex-row sm:items-start sm:justify-between sm:gap-6"
            >
              <div className="flex flex-col gap-0.5">
                <span className="text-sm font-medium">{title}</span>
                <span className="text-sm text-muted-foreground">{detail}</span>
              </div>
              <Badge variant={statusBadge[status].variant} className="shrink-0">
                {statusBadge[status].label}
              </Badge>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  )
}
