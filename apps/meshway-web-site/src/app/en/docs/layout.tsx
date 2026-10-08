import type { ReactNode } from "react"

import { DocsShell } from "@/components/site/docs"

export default function Layout({ children }: { children: ReactNode }) {
  return <DocsShell lang="en">{children}</DocsShell>
}
