import { HomeLayout } from "fumadocs-ui/layouts/home"
import type { ReactNode } from "react"

import { homeOptions } from "@/components/site/layout"

export default function Layout({ children }: { children: ReactNode }) {
  return <HomeLayout {...homeOptions("en")}>{children}</HomeLayout>
}
