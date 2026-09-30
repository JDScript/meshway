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
    title: "管理员认证",
    detail: "控制台与管理 API 需要管理员令牌；会话保存在仅服务端可读的 Cookie 中。",
    status: "implemented",
  },
  {
    title: "节点注册 · Enrollment",
    detail: "一次性、有时效的注册令牌；agent 注册后持久保存自己的控制面凭据，记录在 controller 中持久存储。",
    status: "implemented",
  },
  {
    title: "心跳与撤销",
    detail: "心跳更新最后联系时间；撤销后 controller 拒绝该节点凭据。不代表在线或连通性。",
    status: "implemented",
  },
  {
    title: "数据面身份",
    detail: "节点密钥与数据面身份证明。当前身份仅为控制面 bearer 凭据。",
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
    title: "Mesh 地址分配",
    detail: "Controller 为每个节点分配 IPv4 与 IPv6 双栈 Mesh 地址。地址尚未配置到任何网络接口。",
    status: "implemented",
  },
  {
    title: "路由策略编写与编译预览",
    detail: "有序 first-match 规则：来源节点/国家，目标 Mesh 节点、域名后缀、CIDR、国家，有序经由节点与显式出口；默认动作 direct/reject，初始为 reject。编译预览给出规划路径，不是实际 trace；按配置版本条件保存。",
    status: "implemented",
  },
  {
    title: "配置下发与确认",
    detail: "节点拉取配置并确认已校验保存的版本。确认不代表网络已生效。",
    status: "implemented",
  },
  {
    title: "按策略转发与必经节点",
    detail: "节点按编译后的逻辑路径实际转发流量；传输切换不得绕过必经节点。",
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
