import { CircleAlertIcon, InfoIcon } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
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
          指向正确的地址。
        </p>
        <pre className="overflow-x-auto rounded-md bg-destructive/5 px-3 py-2 font-mono text-xs whitespace-pre-wrap">
          {error}
        </pre>
      </AlertDescription>
    </Alert>
  )
}

export function NoNodes({ url }: { url: string }) {
  const command = `cargo run -p meshway-agent -- report --controller ${url} --name nyc --node-id <uuid>`

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">还没有节点上报</CardTitle>
        <CardDescription className="text-sm">
          Controller 可以访问，但尚未收到任何 agent 上报。在节点上运行一次开发用上报命令，把{" "}
          <code className="font-mono text-xs">&lt;uuid&gt;</code>{" "}
          换成该节点固定使用的 UUID：
        </CardDescription>
      </CardHeader>
      <CardContent>
        <pre className="overflow-x-auto rounded-md border bg-muted/50 px-3 py-2.5 font-mono text-xs leading-relaxed">
          {command}
        </pre>
      </CardContent>
    </Card>
  )
}

export function DevelopmentNotice() {
  return (
    <Alert className="px-4 py-3 text-sm">
      <InfoIcon />
      <AlertTitle className="text-sm">开发阶段：仅有进程上报</AlertTitle>
      <AlertDescription className="text-sm">
        这里的节点来自 agent 的开发用 report 请求。目前没有安全注册（enrollment）、没有 TUN
        数据面，也没有建立隧道。一条上报记录只说明 controller 收到过该进程的请求，不代表节点在线或
        mesh 已连通。
      </AlertDescription>
    </Alert>
  )
}
