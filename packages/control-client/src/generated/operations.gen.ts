// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import { request } from "../transport.ts";
import type { AcknowledgeAgentConnectionsRequest, AcknowledgeAgentConnectionsResponse, AcknowledgeConfigurationRequest, AcknowledgeConfigurationResponse, AgentConnectionsSnapshot, AgentNodeResponse, ConfigurationSnapshot, ConnectionsSnapshot, CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse, EnrollAgentRequest, HeartbeatRequest, NodeList, Overview, PolicySnapshot, PreviewPolicyRequest, PreviewPolicyResponse, PublishConnectionIdentityRequest, PublishConnectionIdentityResponse, RevokeNodeRequest, RevokeNodeResponse, SaveConnectionsRequest, SavePolicyRequest, SetNodeCountryRequest } from "./types.gen.ts";
import { zAcknowledgeAgentConnectionsRequest, zAcknowledgeAgentConnectionsResponse, zAcknowledgeConfigurationRequest, zAcknowledgeConfigurationResponse, zAgentConnectionsSnapshot, zAgentNodeResponse, zConfigurationSnapshot, zConnectionsSnapshot, zCreateEnrollmentTokenRequest, zCreateEnrollmentTokenResponse, zEnrollAgentRequest, zHeartbeatRequest, zNodeList, zOverview, zPolicySnapshot, zPreviewPolicyRequest, zPreviewPolicyResponse, zPublishConnectionIdentityRequest, zPublishConnectionIdentityResponse, zRevokeNodeRequest, zRevokeNodeResponse, zSaveConnectionsRequest, zSavePolicyRequest, zSetNodeCountryRequest } from "./zod.gen.ts";

export async function publishConnectionIdentityOperation(baseUrl: string, token: string, body: PublishConnectionIdentityRequest): Promise<PublishConnectionIdentityResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/connection-identity", token, body: zPublishConnectionIdentityRequest.parse(body) }, zPublishConnectionIdentityResponse);
}

export async function getAgentConnectionsOperation(baseUrl: string, token: string): Promise<AgentConnectionsSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agent/connections", token }, zAgentConnectionsSnapshot);
}

export async function acknowledgeAgentConnectionsOperation(baseUrl: string, token: string, body: AcknowledgeAgentConnectionsRequest): Promise<AcknowledgeAgentConnectionsResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/connections/ack", token, body: zAcknowledgeAgentConnectionsRequest.parse(body) }, zAcknowledgeAgentConnectionsResponse);
}

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

export async function getConnectionsOperation(baseUrl: string, token: string): Promise<ConnectionsSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/connections", token }, zConnectionsSnapshot);
}

export async function saveConnectionsOperation(baseUrl: string, token: string, body: SaveConnectionsRequest): Promise<ConnectionsSnapshot> {
  return request(baseUrl, { method: "POST", path: "/api/v1/connections", token, body: zSaveConnectionsRequest.parse(body) }, zConnectionsSnapshot);
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
