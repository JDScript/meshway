// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import { request } from "../transport.ts";
import type { AcknowledgeConfigurationRequest, AcknowledgeConfigurationResponse, AgentNodeResponse, ConfigurationSnapshot, CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse, EnrollAgentRequest, HeartbeatRequest, NodeList, Overview, PolicySnapshot, PreviewPolicyRequest, PreviewPolicyResponse, RevokeNodeRequest, RevokeNodeResponse, SavePolicyRequest, SetNodeCountryRequest } from "./types.gen.ts";
import { zAcknowledgeConfigurationRequest, zAcknowledgeConfigurationResponse, zAgentNodeResponse, zConfigurationSnapshot, zCreateEnrollmentTokenRequest, zCreateEnrollmentTokenResponse, zEnrollAgentRequest, zHeartbeatRequest, zNodeList, zOverview, zPolicySnapshot, zPreviewPolicyRequest, zPreviewPolicyResponse, zRevokeNodeRequest, zRevokeNodeResponse, zSavePolicyRequest, zSetNodeCountryRequest } from "./zod.gen.ts";

export async function getAgentConfigurationOperation(baseUrl: string, token: string): Promise<ConfigurationSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agents/configuration", token }, zConfigurationSnapshot);
}

export async function acknowledgeConfigurationOperation(baseUrl: string, token: string, body: AcknowledgeConfigurationRequest): Promise<AcknowledgeConfigurationResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agents/configuration/ack", token, body: zAcknowledgeConfigurationRequest.parse(body) }, zAcknowledgeConfigurationResponse);
}

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

export async function setNodeCountryOperation(baseUrl: string, token: string, body: SetNodeCountryRequest): Promise<RevokeNodeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/nodes/country", token, body: zSetNodeCountryRequest.parse(body) }, zRevokeNodeResponse);
}

export async function revokeNodeOperation(baseUrl: string, token: string, body: RevokeNodeRequest): Promise<RevokeNodeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/nodes/revoke", token, body: zRevokeNodeRequest.parse(body) }, zRevokeNodeResponse);
}

export async function getOverviewOperation(baseUrl: string, token: string): Promise<Overview> {
  return request(baseUrl, { method: "GET", path: "/api/v1/overview", token }, zOverview);
}

export async function getPolicyOperation(baseUrl: string, token: string): Promise<PolicySnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/policy", token }, zPolicySnapshot);
}

export async function previewPolicyOperation(baseUrl: string, token: string, body: PreviewPolicyRequest): Promise<PreviewPolicyResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/policy/preview", token, body: zPreviewPolicyRequest.parse(body) }, zPreviewPolicyResponse);
}

export async function savePolicyOperation(baseUrl: string, token: string, body: SavePolicyRequest): Promise<PolicySnapshot> {
  return request(baseUrl, { method: "POST", path: "/api/v1/policy/save", token, body: zSavePolicyRequest.parse(body) }, zPolicySnapshot);
}
