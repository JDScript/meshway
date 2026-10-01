import { proxyNodeRequest } from "@/lib/node-proxy"

export const dynamic = "force-dynamic"
export const runtime = "nodejs"
export function POST(request: Request): Promise<Response> {
  return proxyNodeRequest(request, "/api/v1/devices/exchange")
}
