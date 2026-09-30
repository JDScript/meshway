import type { Metadata } from "next"
import { connection } from "next/server"

import { EnrollmentPanel } from "@/components/enrollment/enrollment-panel"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { requireAdminToken } from "@/lib/session"

export const metadata: Metadata = { title: "注册令牌" }

export default async function EnrollmentPage() {
  await connection()
  await requireAdminToken()

  return (
    <PageBody>
      <PageHeader
        title="注册令牌"
        description="为新节点创建一次性注册令牌，并生成在节点上执行的注册命令。"
      />
      <EnrollmentPanel />
    </PageBody>
  )
}
