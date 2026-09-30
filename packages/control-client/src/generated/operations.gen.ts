// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import { request } from "../transport.ts";
import type { AgentNodeResponse, CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse, EnrollAgentRequest, HeartbeatRequest, NodeList, Overview, RevokeNodeRequest, RevokeNodeResponse } from "./types.gen.ts";
import { zAgentNodeResponse, zCreateEnrollmentTokenRequest, zCreateEnrollmentTokenResponse, zEnrollAgentRequest, zHeartbeatRequest, zNodeList, zOverview, zRevokeNodeRequest, zRevokeNodeResponse } from "./zod.gen.ts";

export async function enrollAgentOperation(baseUrl: string, token: string, body: EnrollAgentRequest): Promise<AgentNodeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agents/enroll", token, body: zEnrollAgentRequest.parse(body) }, zAgentNodeResponse);
}

export async function heartbeatAgentOperation(baseUrl: string, token: string, body: HeartbeatRequest): Promise<AgentNodeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agents/heartbeat", token, body: zHeartbeatRequest.parse(body) }, zAgentNodeResponse);
}

export async function createEnrollmentTokenOperation(baseUrl: string, token: string, body: CreateEnrollmentTokenRequest): Promise<CreateEnrollmentTokenResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/enrollment-tokens", token, body: zCreateEnrollmentTokenRequest.parse(body) }, zCreateEnrollmentTokenResponse);
}

export async function listNodesOperation(baseUrl: string, token: string): Promise<NodeList> {
  return request(baseUrl, { method: "GET", path: "/api/v1/nodes", token }, zNodeList);
}

export async function revokeNodeOperation(baseUrl: string, token: string, body: RevokeNodeRequest): Promise<RevokeNodeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/nodes/revoke", token, body: zRevokeNodeRequest.parse(body) }, zRevokeNodeResponse);
}

export async function getOverviewOperation(baseUrl: string, token: string): Promise<Overview> {
  return request(baseUrl, { method: "GET", path: "/api/v1/overview", token }, zOverview);
}
