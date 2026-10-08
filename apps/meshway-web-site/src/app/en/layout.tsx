import type { Metadata } from "next"
import type { ReactNode } from "react"

import { SiteRoot } from "@/components/site/root"
import { appName } from "@/lib/site"

export const metadata: Metadata = {
  title: { default: appName, template: `%s · ${appName}` },
}

export default function Layout({ children }: { children: ReactNode }) {
  return <SiteRoot lang="en">{children}</SiteRoot>
}
