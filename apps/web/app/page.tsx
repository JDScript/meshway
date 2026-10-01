import { NetworkLanding } from "@/components/shell/legacy-redirect"

export default function RootPage() {
  return <NetworkLanding target={(routes) => routes.overview} />
}
