import type { Metadata } from "next"

import { SignInCallback } from "@/components/try/callback"

export const metadata: Metadata = { robots: { index: false } }

export default function Page() {
  return <SignInCallback lang="en" />
}
