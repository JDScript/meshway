import type { Metadata } from "next"
import { redirect } from "next/navigation"
import { listNetworks, type NetworkList } from "@meshway/control-client"
import { DeviceAuthorizationForm } from "@/components/auth/device-authorization-form"
import { controllerUrl, isCredentialRejected } from "@/lib/controller"
import { authorizationPath, parseDeviceAuthorization } from "@/lib/device-login"
import { readSession } from "@/lib/session"

export const metadata: Metadata = { title: "授权设备", robots: { index: false, follow: false }, referrer: "no-referrer" }

export default async function AuthorizeDevicePage({ searchParams }: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const value = parseDeviceAuthorization(await searchParams)
  const frame = (content: React.ReactNode) => <main className="flex min-h-svh items-center justify-center px-4 py-12"><section className="flex w-full max-w-md flex-col gap-5 rounded-xl border bg-card p-6">{content}</section></main>
  if (!value) return frame(<><h1 className="text-xl font-semibold">授权请求无效</h1><p className="text-sm text-muted-foreground">回调地址或请求参数不受支持。请从 Meshway App 重新发起登录。</p></>)
  const loginPath = `/login?returnTo=${encodeURIComponent(authorizationPath(value))}`
  const session = await readSession()
  if (session.kind !== "active") redirect(loginPath)
  let networks: NetworkList
  try {
    networks = await listNetworks(controllerUrl(), session.token)
  } catch (error) {
    if (isCredentialRejected(error)) redirect(`${loginPath}&reason=expired`)
    return frame(<><h1 className="text-xl font-semibold">无法读取 Controller</h1><p className="text-sm">授权尚未完成，请稍后重试。</p></>)
  }
  return frame(<>
    <h1 className="text-xl font-semibold">授权设备接入</h1>
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
      <dt>设备</dt><dd className="break-all">{value.request.name}</dd>
      <dt>版本</dt><dd className="break-all">{value.request.agent_version}</dd>
      <dt>Controller</dt><dd className="break-all">{controllerUrl()}</dd>
    </dl>
    <p className="text-sm text-muted-foreground">批准后 App 可兑换一个节点身份。实际访问由网络策略决定，接入不代表隧道已建立。</p>
    <DeviceAuthorizationForm value={value} networks={networks} />
  </>)
}
