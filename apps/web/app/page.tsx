import { NetworkLanding } from "@/components/shell/legacy-redirect"

export default function RootPage() {
  return <NetworkLanding root target={(routes) => routes.overview} />
}
