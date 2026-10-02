import type { Metadata } from "next"
import Link from "next/link"

import { AccountWorkspace } from "@/components/account/account-workspace"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { fetchConsoleIdentity } from "@/lib/account-session"
import { fetchNodeOwnership, fetchNodes } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"

export const metadata: Metadata = { title: "我的账户", referrer: "no-referrer" }

export default async function AccountPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [scope, identity, query] = await Promise.all([requireNetwork(params), fetchConsoleIdentity(), searchParams])
  if (identity.kind !== "user") {
    return (
      <PageBody>
        <PageHeader title="我的账户" />
        {identity.kind === "bootstrap" ? (
          <p className="text-[0.8125rem] text-muted-foreground">
            当前是实例管理员令牌会话，它不是任何人的账户。账户恢复与会话撤销在{" "}
            <Link href="/setup" className="underline underline-offset-4">实例初始化与恢复</Link> 中进行。
          </p>
        ) : (
          <p role="alert" className="text-[0.8125rem] text-destructive">暂时无法读取账户：{identity.error}</p>
        )}
      </PageBody>
    )
  }
  const membership = identity.memberships.find((entry) => entry.networkId === scope.networkId) ?? null
  const [ownership, nodes] = await Promise.all([fetchNodeOwnership(scope.networkId), fetchNodes(scope.networkId)])
  const mine = ownership.ok && nodes.ok && membership
    ? { ok: true as const, nodes: nodes.data.nodes.filter((node) => ownership.data.nodes.some((entry) => entry.node_id === node.id && entry.membership_id === membership.membershipId)) }
    : { ok: false as const, error: !ownership.ok ? ownership.error : !nodes.ok ? nodes.error : "你不是本网络成员。" }
  const names = new Map(scope.networks.ok ? scope.networks.data.networks.map((network) => [network.network_id, network.name]) : [])

  return (
    <PageBody>
      <AccountWorkspace
        email={identity.email}
        emailVerified={identity.emailVerified}
        memberships={identity.memberships.map((entry) => ({ networkId: entry.networkId, role: entry.role, name: names.get(entry.networkId) ?? null }))}
        currentNetwork={scope.networkId}
        devices={mine}
        fetchedAt={nodes.fetchedAt}
        changed={query.changed === "password"}
      />
    </PageBody>
  )
}
