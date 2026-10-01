import { proxyNodeRequest } from "@/lib/node-proxy"

export const dynamic = "force-dynamic"
export function GET(request: Request) {
  return proxyNodeRequest(request, "/api/v1/agents/configuration")
}
