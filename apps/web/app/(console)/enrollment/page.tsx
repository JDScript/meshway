import type { Metadata } from "next"
import { connection } from "next/server"

import { EnrollmentPanel } from "@/components/enrollment-panel"
import { PageHeader } from "@/components/page-header"
import { requireAdminToken } from "@/lib/session"

export const metadata: Metadata = { title: "注册令牌" }

export default async function EnrollmentPage() {
  await connection()
  await requireAdminToken()

  return (
    <>
      <PageHeader
        title="注册令牌"
        description="为新节点创建一次性注册令牌。令牌只在创建时显示一次，使用一次后即失效；过期未用也会失效。"
      />
      <EnrollmentPanel />
    </>
  )
}
