import type { ReactNode } from "react"

import { SectionTabs } from "@/components/shell/section-tabs"

export default function DiagnosticsLayout({ children }: { children: ReactNode }) {
  return (
    <>
      <div data-section-tabs className="mx-auto w-full max-w-[76rem] px-4 pt-6 md:px-8">
        <SectionTabs group="diagnostics" />
      </div>
      {children}
    </>
  )
}
