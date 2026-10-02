import {
  listAccountMembers,
  listAccountPasswordResets,
  listNetworks,
  type AccountPasswordReset,
  type AccountRole,
} from "@meshway/control-client"
import { KeyRoundIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"
import { redirect } from "next/navigation"
import { connection } from "next/server"

import { LoginForm } from "@/components/auth/login-form"
import { SwitchAccountButton } from "@/components/auth/switch-account-button"
import { SetupAccounts, type SetupAccount } from "@/components/setup/setup-accounts"
import { SetupNetworks, type SetupNetwork } from "@/components/setup/setup-networks"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { EXPIRED_LOGIN_PATH, readSession, sessionKey } from "@/lib/session"

export const metadata: Metadata = { title: "实例初始化与恢复", robots: { index: false, follow: false }, referrer: "no-referrer" }

/**
 * Instance setup and operator recovery, the only place the administrator
 * token is used: create networks, give a network its first Owner, and help
 * an account that lost its password or a device. Daily work is done with a
 * user account; network roles never grant any of this.
 */
export default async function SetupPage() {
  await connection()
  const session = await readSession()
  if (session.kind !== "active" || session.credential.kind !== "bootstrap") {
    const signedInAsUser = session.kind === "active" && session.credential.kind === "user"
    return (
      <StandaloneFrame
        width="md"
        footer={<Link href={signedInAsUser ? "/" : "/login"} className="underline-offset-4 hover:text-foreground hover:underline">{signedInAsUser ? "返回控制台" : "返回账户登录"}</Link>}
      >
        <div className="flex flex-col gap-1">
          <span className="flex size-9 items-center justify-center rounded-lg bg-muted text-muted-foreground">
            <KeyRoundIcon className="size-4.5" aria-hidden="true" />
          </span>
          <h1 className="mt-2 text-xl font-semibold tracking-tight">实例初始化与恢复</h1>
          <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
            首次部署时创建网络并指定第一个所有者；之后只在账户无法登录时使用。日常管理请用账户登录。
          </p>
        </div>
        {signedInAsUser ? (
          <p role="status" className="rounded-md border bg-muted/40 px-3 py-2 text-xs leading-relaxed text-muted-foreground">
            你当前以账户登录。使用实例管理员令牌会先退出该账户。
          </p>
        ) : null}
        <LoginForm />
        <details className="group rounded-md border text-xs">
          <summary className="cursor-pointer list-none px-3 py-2 font-medium text-muted-foreground select-none hover:text-foreground [&::-webkit-details-marker]:hidden">
            在哪里找到令牌
          </summary>
          <div className="flex flex-col gap-2 px-3 pb-3 text-muted-foreground">
            <span>令牌只在运行 controller 的机器上读取，不会在网页中显示或保存到账户：</span>
            <pre className="overflow-x-auto rounded-md border bg-muted/50 px-3 py-2 font-mono text-[0.6875rem] text-foreground">
              meshway-controller admin-token --data-dir &lt;数据目录&gt;
            </pre>
          </div>
        </details>
      </StandaloneFrame>
    )
  }

  const token = session.credential.token
  const signIn = await sessionKey()
  let networks: SetupNetwork[]
  let resets: AccountPasswordReset[]
  let now: number
  try {
    const [listed, resetList] = await Promise.all([listNetworks(controllerUrl(), token), listAccountPasswordResets(controllerUrl(), token)])
    resets = resetList.resets
    now = readClock()
    networks = await Promise.all(
      listed.networks.map(async (network): Promise<SetupNetwork> => {
        try {
          const { members } = await listAccountMembers(controllerUrl(), token, network.network_id)
          return {
            networkId: network.network_id,
            name: network.name,
            prefix: network.network.ipv4_prefix,
            members: { ok: true, list: members.map((member) => ({ userId: member.user.user_id, email: member.user.email, role: member.membership.role })) },
          }
        } catch (error) {
          if (isCredentialRejected(error)) throw error
          return { networkId: network.network_id, name: network.name, prefix: network.network.ipv4_prefix, members: { ok: false, error: describeError(error, token) } }
        }
      })
    )
  } catch (error) {
    if (isCredentialRejected(error)) redirect(EXPIRED_LOGIN_PATH)
    return (
      <StandaloneFrame width="md">
        <h1 className="text-lg font-semibold tracking-tight">无法读取实例状态</h1>
        <pre className="max-w-full rounded-md border bg-muted/50 px-3 py-2 font-mono text-[0.6875rem] whitespace-pre-wrap [overflow-wrap:anywhere]">
          {describeError(error, token)}
        </pre>
      </StandaloneFrame>
    )
  }

  // Accounts are global; the controller lists them only per network, so an
  // account appears here once it belongs to at least one network.
  const accounts = new Map<string, SetupAccount>()
  for (const network of networks) {
    if (!network.members.ok) continue
    for (const member of network.members.list) {
      const memberships: { name: string; role: AccountRole }[] = []
      const account = accounts.get(member.userId) ?? { userId: member.userId, email: member.email, networks: memberships }
      account.networks.push({ name: network.name, role: member.role })
      accounts.set(member.userId, account)
    }
  }
  const unclaimed = networks.filter((network) => network.members.ok && !network.members.list.some((member) => member.role === "owner"))

  return (
    <StandaloneFrame width="lg">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex flex-col gap-1">
          <h1 className="text-xl font-semibold tracking-tight">实例初始化与恢复</h1>
          <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
            以实例管理员令牌登录，8 小时内有效。它不是任何人的账户，不能批准设备，也不出现在网络成员中。
          </p>
        </div>
        <div className="shrink-0 sm:w-40">
          <SwitchAccountButton signIn={signIn} returnTo="/" label="退出" />
        </div>
      </div>
      {unclaimed.length > 0 ? (
        <p role="status" className="rounded-md border border-warning/40 bg-warning/8 px-3 py-2 text-[0.8125rem]">
          {unclaimed.length} 个网络还没有所有者。认领后，所有者用账户登录即可邀请成员、认领现有节点。
        </p>
      ) : null}
      <SetupNetworks signIn={signIn} networks={networks} />
      <SetupAccounts signIn={signIn} accounts={[...accounts.values()].sort((a, b) => a.email.localeCompare(b.email))} resets={resets} now={now} />
    </StandaloneFrame>
  )
}

/** When the reset inventory was read, in the controller's Unix seconds. */
function readClock(): number {
  return Math.floor(Date.now() / 1000)
}
