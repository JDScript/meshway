"use client"

import { ArrowUpRightIcon, CheckIcon, CircleDashedIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover"
import { docsUrl } from "@/lib/config"

// The single place that states what this build does and does not do, so pages
// do not repeat long disclaimers. Keep it in sync with docs/development.md.
const available = [
  "管理员会话、一次性注册令牌与节点撤销",
  "双栈 Mesh 地址分配",
  "路由策略编辑、编译预览与条件保存",
  "node / tag / * 端点授权、双向规则与条件保存",
  "图形化 / schema JSONC 策略编辑与已保存 Flow 检查",
  "配置下发，节点确认已校验保存的版本",
  "稳定 Network ID、网络名称和节点 DNS 配置",
  "可选本地 UDP / TCP 节点 DNS（A / AAAA / PTR，不接管系统 DNS）",
  "可选 Linux IPv4 托管数据面（agent opt-in）；实际连通需用 ping / curl 单独测试",
]

const pending = [
  "IPv6 TUN、互联网出口与可信域名流量上下文",
  "端到端流量统计与真实多跳 mesh trace",
  "系统 DNS 集成、自动打洞与完整多传输故障切换",
]

export function CapabilityPopover() {
  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button
            variant="outline"
            size="sm"
            className="gap-1.5 rounded-full font-normal text-muted-foreground"
          />
        }
      >
        <span aria-hidden="true" className="size-1.5 rounded-full bg-brand" />
        控制面
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80">
        <PopoverHeader>
          <PopoverTitle>当前能力</PopoverTitle>
          <PopoverDescription>
            控制台展示控制面意图与记录。「最后联系」「配置确认」「规划路径」都不代表节点在线、链路可达或流量已按策略转发。
          </PopoverDescription>
        </PopoverHeader>
        <div className="grid gap-3 text-xs">
          <CapabilityList title="可用" items={available} done />
          <CapabilityList title="尚未实现" items={pending} done={false} />
        </div>
        <a
          href={docsUrl}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
        >
          设计文档
          <ArrowUpRightIcon className="size-3" />
        </a>
      </PopoverContent>
    </Popover>
  )
}

function CapabilityList({
  title,
  items,
  done,
}: {
  title: string
  items: string[]
  done: boolean
}) {
  const Icon = done ? CheckIcon : CircleDashedIcon
  return (
    <div className="grid gap-1.5">
      <span className="text-[0.6875rem] font-medium text-muted-foreground">{title}</span>
      <ul className="grid gap-1">
        {items.map((item) => (
          <li key={item} className="flex items-start gap-2">
            <Icon
              aria-hidden="true"
              className={done ? "mt-0.5 size-3.5 text-brand" : "mt-0.5 size-3.5 text-muted-foreground"}
            />
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </div>
  )
}
