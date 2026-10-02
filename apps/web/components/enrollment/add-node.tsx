"use client"

import { CableIcon, LaptopIcon, ServerIcon } from "lucide-react"
import Link from "next/link"
import { useState, type ReactNode } from "react"

import { Panel } from "@/components/common/panel"
import { EnrollmentPanel } from "@/components/enrollment/enrollment-panel"
import { useRoutes } from "@/components/shell/network-context"
import { cn } from "@/lib/utils"

type Method = "agent" | "app"

/**
 * Two real ways into a network: a server runs `meshway-agent enroll` with a
 * one-time token; the Meshway App signs in through the browser approval page.
 * Both produce the same kind of node; there is no node class to choose.
 */
export function AddNode({ existingListenerIds }: { existingListenerIds: string[] | null }) {
  const [method, setMethod] = useState<Method>("agent")
  const routes = useRoutes()
  return (
    <div className="flex flex-col gap-6">
      <div role="radiogroup" aria-label="接入方式" className="grid gap-3 sm:grid-cols-2">
        <MethodCard
          selected={method === "agent"}
          onSelect={() => setMethod("agent")}
          icon={<ServerIcon />}
          title="服务器 · meshway-agent"
          description="生成一次性接入令牌，在 Linux / macOS 主机上注册并运行 agent。"
        />
        <MethodCard
          selected={method === "app"}
          onSelect={() => setMethod("app")}
          icon={<LaptopIcon />}
          title="Meshway App"
          description="在 App 中登录，系统浏览器打开本控制台的授权页，批准后加入所选网络。"
        />
      </div>

      {method === "agent" ? (
        <EnrollmentPanel existingListenerIds={existingListenerIds} />
      ) : (
        <Panel title="通过 Meshway App 接入">
          <ol className="flex flex-col gap-4 p-5 text-xs leading-relaxed text-muted-foreground">
            <AppStep index={1} title="在 App 中选择登录">App 使用系统浏览器打开此 controller 的授权页，请求中带有设备名与 App 版本。</AppStep>
            <AppStep index={2} title="在浏览器中批准">登录控制台后选择要加入的网络并批准；拒绝不会创建任何节点。</AppStep>
            <AppStep index={3} title="App 完成兑换">
              App 用一次性兑换码与本机保存的节点身份完成接入，节点随后出现在
              <Link href={routes.machines()} className="mx-1 text-foreground underline underline-offset-4">节点</Link>
              列表。接入只代表控制面身份，不代表隧道已建立。
            </AppStep>
          </ol>
          <p className="border-t px-5 py-3 text-xs text-muted-foreground">
            当前由管理员批准暂代用户登录；用户账户与邀请加入上线后，设备将归属登录它的用户。
          </p>
        </Panel>
      )}

      <div className="flex items-start gap-3 rounded-lg border border-dashed bg-card/60 px-4 py-3.5 text-xs leading-relaxed text-muted-foreground">
        <CableIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
        <p>
          <span className="font-medium text-foreground">监听与承载：</span>
          注册后在节点「配置」中查看与调整。
        </p>
      </div>
    </div>
  )
}

function MethodCard({
  selected,
  onSelect,
  icon,
  title,
  description,
}: {
  selected: boolean
  onSelect: () => void
  icon: ReactNode
  title: string
  description: string
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={cn(
        "flex items-start gap-3 rounded-lg border bg-card p-4 text-left shadow-xs transition-colors outline-none hover:border-foreground/20 focus-visible:ring-2 focus-visible:ring-ring/40",
        selected && "border-brand ring-1 ring-brand"
      )}
    >
      <span
        className={cn(
          "flex size-9 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground [&_svg]:size-4.5",
          selected && "bg-brand/12 text-brand"
        )}
      >
        {icon}
      </span>
      <span className="flex min-w-0 flex-col gap-1">
        <span className="text-[0.8125rem] font-semibold">{title}</span>
        <span className="text-xs leading-relaxed text-muted-foreground">{description}</span>
      </span>
    </button>
  )
}

function AppStep({ index, title, children }: { index: number; title: string; children: ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-brand/12 font-mono text-[0.6875rem] font-semibold text-brand">
        {index}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1 pt-0.5">
        <span className="text-[0.8125rem] font-medium text-foreground">{title}</span>
        <span>{children}</span>
      </div>
    </li>
  )
}
