// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import { request } from "../transport.ts";
import type { AgentReport, NodeList, Overview, ReportResponse } from "./types.gen.ts";
import { zAgentReport, zNodeList, zOverview, zReportResponse } from "./zod.gen.ts";

export async function reportAgentOperation(baseUrl: string, body: AgentReport): Promise<ReportResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agents/report", body: zAgentReport.parse(body) }, zReportResponse);
}

export async function listNodesOperation(baseUrl: string): Promise<NodeList> {
  return request(baseUrl, { method: "GET", path: "/api/v1/nodes" }, zNodeList);
}

export async function getOverviewOperation(baseUrl: string): Promise<Overview> {
  return request(baseUrl, { method: "GET", path: "/api/v1/overview" }, zOverview);
}
