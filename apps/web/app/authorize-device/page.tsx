import { listNetworks, type NetworkList } from "@meshway/control-client"
import { CircleAlertIcon, LaptopIcon } from "lucide-react"
import type { Metadata } from "next"
import { redirect } from "next/navigation"
import type { ReactNode } from "react"

import { DeviceAuthorizationForm } from "@/components/auth/device-authorization-form"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { controllerUrl, isCredentialRejected } from "@/lib/controller"
import { authorizationPath, parseDeviceAuthorization } from "@/lib/device-login"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "授权设备", robots: { index: false, follow: false }, referrer: "no-referrer" }

function Problem({ title, children }: { title: string; children: ReactNode }) {
  return (
    <StandaloneFrame width="md">
      <div className="flex flex-col items-center gap-3 text-center">
        <span className="flex size-10 items-center justify-center rounded-full bg-destructive/10 text-destructive">
          <CircleAlertIcon className="size-5" />
        </span>
        <h1 className="text-lg font-semibold tracking-tight">{title}</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">{children}</p>
      </div>
    </StandaloneFrame>
  )
}

/**
 * Browser half of the Apple App sign-in: the App opens this page in the
 * system browser; an administrator (until user accounts exist) approves it
 * into one network and the controller returns a one-time exchange code to the
 * App callback. Nothing here claims a tunnel or data-plane state.
 */
export default async function AuthorizeDevicePage({ searchParams }: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const value = parseDeviceAuthorization(await searchParams)
  if (!value) {
    return <Problem title="授权请求无效">回调地址或请求参数不受支持。请从 Meshway App 重新发起登录。</Problem>
  }
  const loginPath = `/login?returnTo=${encodeURIComponent(authorizationPath(value))}`
  const session = await readSession()
  if (session.kind !== "active") redirect(loginPath)
  let networks: NetworkList
  try {
    networks = await listNetworks(controllerUrl(), session.token)
  } catch (error) {
    if (isCredentialRejected(error)) redirect(`${loginPath}&reason=expired`)
    return <Problem title="无法读取 Controller">授权尚未完成，请稍后重试。</Problem>
  }
  return (
    <StandaloneFrame
      width="md"
      footer={<span>批准后 App 可兑换一个节点身份。实际访问由网络策略决定，接入不代表隧道已建立。</span>}
    >
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">授权设备接入</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          Meshway App 请求以下设备加入网络。只批准你刚刚亲自发起的请求。
        </p>
      </div>
      <div className="flex items-start gap-3 rounded-lg border bg-muted/40 p-3.5">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-md bg-card text-muted-foreground shadow-xs">
          <LaptopIcon className="size-4.5" />
        </span>
        <dl className="grid min-w-0 flex-1 grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-[0.8125rem]">
          <dt className="text-muted-foreground">设备</dt>
          <dd className="font-medium break-all">{value.request.name}</dd>
          <dt className="text-muted-foreground">版本</dt>
          <dd className="font-mono text-xs break-all">{value.request.agent_version}</dd>
          <dt className="text-muted-foreground">Controller</dt>
          <dd className="font-mono text-xs break-all">{controllerUrl()}</dd>
        </dl>
      </div>
      <DeviceAuthorizationForm value={value} networks={networks} />
    </StandaloneFrame>
  )
}
