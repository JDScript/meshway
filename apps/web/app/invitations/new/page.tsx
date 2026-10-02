import { listNetworks, type AccountRole } from "@meshway/control-client"
import type { Metadata } from "next"
import Link from "next/link"
import { notFound, redirect } from "next/navigation"
import { connection } from "next/server"

import { InvitationIssueForm } from "@/components/auth/invitation-issue-form"
import { StandaloneFrame } from "@/components/shell/standalone-frame"
import { fetchConsoleIdentity } from "@/lib/account-session"
import { roleIn } from "@/lib/account-model"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { consoleRoutes, isNetworkId } from "@/lib/routes"
import { EXPIRED_LOGIN_PATH, requireConsoleCredential, sessionKey } from "@/lib/session"

export const metadata: Metadata = { title: "邀请成员", robots: { index: false, follow: false }, referrer: "no-referrer" }

/**
 * Minimal single invitation for one network. Which roles may be granted is
 * only a hint here; the controller enforces the role ceiling.
 */
export default async function NewInvitationPage({ searchParams }: { searchParams: Promise<Record<string, string | string[] | undefined>> }) {
  await connection()
  const { network } = await searchParams
  if (typeof network !== "string" || !isNetworkId(network)) notFound()
  const credential = await requireConsoleCredential()
  const identity = await fetchConsoleIdentity()
  let listed
  try {
    listed = (await listNetworks(controllerUrl(), credential.token)).networks.find((item) => item.network_id === network)
  } catch (error) {
    if (isCredentialRejected(error)) redirect(EXPIRED_LOGIN_PATH)
    return (
      <StandaloneFrame>
        <h1 className="text-lg font-semibold tracking-tight">无法读取网络列表</h1>
        <pre className="max-w-full rounded-md border bg-muted/50 px-3 py-2 font-mono text-[0.6875rem] whitespace-pre-wrap [overflow-wrap:anywhere]">
          {describeError(error, credential.token)}
        </pre>
      </StandaloneFrame>
    )
  }
  // Only networks this credential may see; never another network.
  if (!listed) notFound()
  const name = listed.name
  const role = roleIn(identity, network)
  const roles: AccountRole[] =
    identity.kind === "bootstrap" || role === "owner" ? ["member", "admin", "owner"] : role === "admin" ? ["member"] : []
  return (
    <StandaloneFrame footer={<Link href={consoleRoutes(network).overview} className="underline underline-offset-4">返回「{name}」</Link>}>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-lg font-semibold tracking-tight">邀请加入「{name}」</h1>
        <p className="text-[0.8125rem] leading-relaxed text-muted-foreground">
          受邀人用链接以该邮箱注册，或以已有账户接受。不会发送邮件。
        </p>
      </div>
      {roles.length > 0 ? (
        <InvitationIssueForm signIn={await sessionKey()} networkId={network} roles={roles} />
      ) : (
        <p role="status" className="text-[0.8125rem] text-muted-foreground">只有本网络的所有者或管理员可以邀请成员。</p>
      )}
    </StandaloneFrame>
  )
}
