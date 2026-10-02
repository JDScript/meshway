import { listAccountMembers, listNetworks, type AccountMembers, type NetworkSnapshot } from "@meshway/control-client"
import type { Metadata } from "next"
import Link from "next/link"
import { redirect } from "next/navigation"
import { connection } from "next/server"

import { OwnerClaimForm } from "@/components/auth/owner-claim-form"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { EXPIRED_LOGIN_PATH, requireConsoleCredential } from "@/lib/session"

export const metadata: Metadata = { title: "认领网络所有者", robots: { index: false, follow: false }, referrer: "no-referrer" }

type Row = { network: NetworkSnapshot; owners: { ok: true; emails: string[] } | { ok: false; error: string } }

/**
 * First-run account bootstrap: with the administrator token, give a network
 * that has no Owner its first Owner account. There is no public sign-up;
 * everyone else joins through invitation links.
 */
export default async function OnboardingPage() {
  await connection()
  const credential = await requireConsoleCredential()
  if (credential.kind !== "bootstrap") {
    return (
      <StandaloneFrame>
        <div className="flex flex-col gap-1.5">
          <h1 className="text-lg font-semibold tracking-tight">认领网络所有者</h1>
          <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
            认领第一个所有者需要使用管理员令牌登录。你当前已用账户登录，可直接进入控制台。
          </p>
        </div>
        <Link href="/" className="text-xs text-muted-foreground underline underline-offset-4">进入控制台</Link>
      </StandaloneFrame>
    )
  }
  const token = credential.token
  let rows: Row[]
  try {
    const networks = await listNetworks(controllerUrl(), token)
    rows = await Promise.all(
      networks.networks.map(async (network): Promise<Row> => {
        let members: AccountMembers
        try {
          members = await listAccountMembers(controllerUrl(), token, network.network_id)
        } catch (error) {
          if (isCredentialRejected(error)) throw error
          return { network, owners: { ok: false, error: describeError(error, token) } }
        }
        const emails = members.members.filter((member) => member.membership.role === "owner").map((member) => member.user.email)
        return { network, owners: { ok: true, emails } }
      })
    )
  } catch (error) {
    if (isCredentialRejected(error)) redirect(EXPIRED_LOGIN_PATH)
    return (
      <StandaloneFrame>
        <h1 className="text-lg font-semibold tracking-tight">无法读取网络列表</h1>
        <pre className="max-w-full rounded-md border bg-muted/50 px-3 py-2 font-mono text-[0.6875rem] whitespace-pre-wrap [overflow-wrap:anywhere]">
          {describeError(error, token)}
        </pre>
      </StandaloneFrame>
    )
  }

  return (
    <StandaloneFrame width="md" footer={<Link href="/" className="underline underline-offset-4">返回控制台</Link>}>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">认领网络所有者</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          为还没有所有者的网络指定第一个所有者账户。请使用该所有者本人的邮箱，并由本人设置密码。现有节点不会自动归属任何账户。
        </p>
      </div>
      {rows.length === 0 ? <p className="text-[0.8125rem] text-muted-foreground">Controller 没有任何网络。</p> : null}
      {rows.map(({ network, owners }) => (
        <section key={network.network_id} className="flex flex-col gap-3 border-t pt-4 first-of-type:border-t-0 first-of-type:pt-0" aria-label={network.name}>
          <div className="flex items-baseline justify-between gap-2">
            <h2 className="truncate text-sm font-semibold">{network.name}</h2>
            <span className="shrink-0 font-mono text-xs text-muted-foreground">{network.network.ipv4_prefix}</span>
          </div>
          {!owners.ok ? (
            <p role="alert" className="text-xs break-words text-destructive">无法读取成员：{owners.error}</p>
          ) : owners.emails.length > 0 ? (
            <p className="text-[0.8125rem] text-muted-foreground">
              已有所有者：<span className="break-all text-foreground">{owners.emails.join("、")}</span>
            </p>
          ) : (
            <OwnerClaimForm networkId={network.network_id} networkName={network.name} />
          )}
        </section>
      ))}
    </StandaloneFrame>
  )
}
