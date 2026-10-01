import { ArrowLeftRightIcon, GaugeIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"

import { Panel } from "@/components/common/panel"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { Badge } from "@/components/ui/badge"
import { buttonVariants } from "@/components/ui/button"

export const metadata: Metadata = { title: "监控 · 预览" }

/** Keep the old URL, while directing implemented checks to their canonical homes. */
export default function MonitoringPage() {
  return (
    <PageBody>
      <PageHeader title="监控" description="测量能力预览。当前没有 Mesh 流量、吞吐或端到端路径延迟的上报数据。" />
      <Panel title="已有的检查入口" description="控制面联系、邻居探测与规划路径各有不同的数据来源。" bodyClassName="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center gap-3">
          <Link href="/nodes" className={buttonVariants({ variant: "outline", size: "sm" })}>节点与控制面联系</Link>
          <Link href="/topology" className={buttonVariants({ variant: "outline", size: "sm" })}>邻接与已保存流量检查</Link>
        </div>
        <p className="text-xs text-muted-foreground">真实邻居探测 RTT 在拓扑及节点连接页展示；不等同于业务流量延迟。规划路径不等同于实际 trace。</p>
      </Panel>
      <div className="grid gap-4 md:grid-cols-2">
        <Panel title={<span className="flex items-center gap-2"><ArrowLeftRightIcon className="size-4" />流量与吞吐<Badge variant="outline">未测量</Badge></span>} bodyClassName="p-4">
          <p className="text-xs text-muted-foreground">按节点与规则统计的流量尚未上报，没有可展示的时间序列。</p>
        </Panel>
        <Panel title={<span className="flex items-center gap-2"><GaugeIcon className="size-4" />Mesh trace<Badge variant="outline">未测量</Badge></span>} bodyClassName="p-4">
          <p className="text-xs text-muted-foreground">实际转发路径测量尚未提供；验证必经节点与出口不能用规划路径代替。</p>
        </Panel>
      </div>
    </PageBody>
  )
}
