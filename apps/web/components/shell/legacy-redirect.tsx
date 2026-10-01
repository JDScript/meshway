import Link from "next/link"
import { redirect } from "next/navigation"
import { connection } from "next/server"

import { RefreshButton } from "@/components/common/refresh-button"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { landingNetwork } from "@/lib/network-scope"
import { consoleRoutes, legacyTarget, type ConsoleRoutes } from "@/lib/routes"
import { requireAdminToken } from "@/lib/session"

type SearchParams = Record<string, string | string[] | undefined>

/**
 * Entry points without a network segment: `/` and the pre-network URLs
 * (`/nodes`, `/routing`, …). They land in the remembered or first network.
 */
export async function NetworkLanding({ target }: { target: (routes: ConsoleRoutes) => string }) {
  await connection()
  await requireAdminToken()
  const landing = await landingNetwork()
  if (landing.ok) redirect(target(consoleRoutes(landing.networkId)))
  return (
    <StandaloneFrame>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">无法读取网络列表</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          控制台需要先从 controller 读取网络，才能打开对应页面。请确认控制面正在运行后重试。
        </p>
        <pre className="mt-2 max-w-full rounded-md border bg-muted/50 px-3 py-2 font-mono text-[0.6875rem] whitespace-pre-wrap [overflow-wrap:anywhere]">
          {landing.error}
        </pre>
      </div>
      <div className="flex items-center justify-between gap-2">
        <Link href="/login" className="text-xs text-muted-foreground underline underline-offset-4">返回登录</Link>
        <RefreshButton />
      </div>
    </StandaloneFrame>
  )
}

export async function LegacyRedirect({
  kind,
  searchParams,
  nodeId,
}: {
  kind: Parameters<typeof legacyTarget>[0]
  searchParams: Promise<SearchParams>
  nodeId?: Promise<{ id: string }>
}) {
  const [params, node] = await Promise.all([searchParams, nodeId])
  return <NetworkLanding target={(routes) => legacyTarget(kind, routes, params, node?.id)} />
}
