import { getAccountSession, listNetworks, type AccountSession, type NetworkList } from "@meshway/control-client"
import { CircleAlertIcon, LaptopIcon, ShieldAlertIcon } from "lucide-react"
import Link from "next/link"
import type { Metadata } from "next"
import { redirect } from "next/navigation"
import type { ReactNode } from "react"

import { DeviceAuthorizationForm } from "@/components/auth/device-authorization-form"
import { SwitchAccountButton } from "@/components/auth/switch-account-button"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { controllerUrl, isCredentialRejected } from "@/lib/controller"
import { authorizationPath, parseDeviceAuthorization } from "@/lib/device-login"
import { readSession, sessionKey } from "@/lib/session"

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
 * system browser; the signed-in user approves it into one of their networks
 * and the controller returns a one-time exchange code to the App callback.
 * The device is then owned by that user's membership. Viewing this page never
 * approves anything. Nothing here claims a tunnel or data-plane state.
 */
export default async function AuthorizeDevicePage({ searchParams }: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const value = parseDeviceAuthorization(await searchParams)
  if (!value) {
    return <Problem title="授权请求无效">回调地址或请求参数不受支持。请从 Meshway App 重新发起登录。</Problem>
  }
  const returnTo = authorizationPath(value)
  const loginPath = `/login?returnTo=${encodeURIComponent(returnTo)}`
  const session = await readSession()
  if (session.kind !== "active") redirect(session.kind === "expired" ? `${loginPath}&reason=expired` : loginPath)
  // The sign-in this page is rendered for; the decision is refused for any other.
  const signIn = await sessionKey()
  if (session.credential.kind !== "user") {
    // The administrator token manages the controller but is not a user, so
    // it cannot own a device. Offer the account sign-in (or first Owner).
    return (
      <StandaloneFrame width="md">
        <div className="flex flex-col items-center gap-3 text-center">
          <span className="flex size-10 items-center justify-center rounded-full bg-amber-500/10 text-amber-600 dark:text-amber-400">
            <ShieldAlertIcon className="size-5" />
          </span>
          <h1 className="text-lg font-semibold tracking-tight">需要账户登录</h1>
          <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
            当前使用的是管理员令牌，它不代表任何用户，不能批准设备。设备会归属批准它的账户，请使用账户登录后继续。
          </p>
        </div>
        <div className="flex flex-col gap-2">
          <SwitchAccountButton signIn={signIn} returnTo={returnTo} label="退出管理员会话并使用账户登录" />
          <Link href="/onboarding" className="text-center text-xs text-muted-foreground underline underline-offset-4">
            还没有账户？为网络认领第一个所有者
          </Link>
        </div>
      </StandaloneFrame>
    )
  }
  let account: AccountSession
  let networks: NetworkList
  try {
    ;[account, networks] = await Promise.all([
      getAccountSession(controllerUrl(), session.credential.token),
      listNetworks(controllerUrl(), session.credential.token),
    ])
  } catch (error) {
    if (isCredentialRejected(error)) redirect(`${loginPath}&reason=expired`)
    return <Problem title="无法读取 Controller">授权尚未完成，请稍后重试。</Problem>
  }
  const roles = new Map(account.memberships.map((membership) => [membership.network_id, membership.role]))
  // The controller lists only this user's networks; keep only live memberships.
  const choices = networks.networks.flatMap((network) => {
    const role = roles.get(network.network_id)
    return role ? [{ networkId: network.network_id, name: network.name, ipv4Prefix: network.network.ipv4_prefix, role }] : []
  })
  return (
    <StandaloneFrame
      width="md"
      footer={<span>批准后 App 可兑换一个归属于你的节点身份。实际访问由网络策略决定，接入不代表隧道已建立。</span>}
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
      <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
        <span className="min-w-0 truncate">
          登录为 <span className="font-medium text-foreground">{account.user.email}</span>
        </span>
        <SwitchAccountButton signIn={signIn} returnTo={returnTo} label="切换账户" variant="link" />
      </div>
      {choices.length === 0 ? (
        <p role="status" className="rounded-md border border-dashed px-3 py-2.5 text-[0.8125rem] text-muted-foreground">
          这个账户还没有加入任何网络。请向网络所有者索取邀请链接。
        </p>
      ) : null}
      <DeviceAuthorizationForm
        signIn={signIn}
        value={value}
        networks={choices}
      />
    </StandaloneFrame>
  )
}
