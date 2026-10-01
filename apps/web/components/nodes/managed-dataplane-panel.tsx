import { ArrowUpRightIcon } from "lucide-react"
import Link from "next/link"

import { CopyButton } from "@/components/common/copy-button"
import { Panel } from "@/components/common/panel"
import { docsUrl } from "@/lib/config"
import { shellCommand } from "@/lib/shell"

const RUN_COMMAND = shellCommand([
  "cargo",
  "run",
  "-p",
  "meshway-agent",
  "--features",
  "managed-connections",
  "--",
  "run",
  "--state=.meshway/agent.json",
  "--apply-dataplane",
])

/**
 * How to opt a node into the managed Linux IPv4 TUN. Static guidance only:
 * there is no dataplane status API, so nothing here describes this node's
 * actual packet state.
 */
export function ManagedDataplanePanel() {
  return (
    <Panel
      title="托管数据面"
      description="可选，仅 Linux IPv4。Agent 须以状态文件属主的同一 UID 运行，并具备 CAP_NET_ADMIN。"
      bodyClassName="flex flex-col gap-2 px-4 py-3 text-xs"
    >
      <div className="relative">
        <pre
          data-testid="node-dataplane-command"
          aria-label="启动参数模板"
          className="overflow-x-auto rounded-lg border bg-muted/40 py-2.5 pr-11 pl-3 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap"
        >
          {RUN_COMMAND}
        </pre>
        <div className="absolute top-1.5 right-1.5">
          <CopyButton value={RUN_COMMAND} label="复制参数模板" iconOnly variant="ghost" />
        </div>
      </div>
      <p className="text-muted-foreground">
        这是参数模板：把 <code className="font-mono text-foreground">--apply-dataplane</code>{" "}
        加到该节点现有的 agent 启动配置里，沿用它自己的 <code className="font-mono">--state</code>（VLESS 另需本地{" "}
        <code className="font-mono">--connection-profiles</code>），不要另起第二个 agent；持久运行的 Docker 演示请按文档启用。流量还须被
        <Link href="/access" className="mx-0.5 underline underline-offset-4">
          访问授权
        </Link>
        允许，并按路由选中的路径转发；连通请用 ping / curl 实测。
      </p>
      <a
        href={`${docsUrl}/managed-dataplane`}
        target="_blank"
        rel="noreferrer"
        className="inline-flex items-center gap-1 self-start text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
      >
        托管数据面与 Docker 演示启用
        <ArrowUpRightIcon className="size-3" />
      </a>
    </Panel>
  )
}
