// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import { request } from "../transport.ts";
import type { AcknowledgeAgentConnectionsRequest, AcknowledgeAgentConnectionsResponse, AcknowledgeConfigurationRequest, AcknowledgeConfigurationResponse, AgentConnectionsSnapshot, AgentDataplaneSnapshot, AgentDnsSnapshot, AgentNodeResponse, AuthorizeDeviceRequest, AuthorizeDeviceResponse, BeginConnectionRuntimeRequest, BeginConnectionRuntimeResponse, ConfigurationSnapshot, ConnectionRuntimesSnapshot, ConnectionsSnapshot, CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse, EndpointDiscoveriesSnapshot, EnrollAgentRequest, ExchangeDeviceRequest, GetConnectionRuntimeEpochResponse, HeartbeatRequest, MeshAccessSnapshot, NetworkSnapshot, NodeList, Overview, PolicySnapshot, PreviewFlowRequest, PreviewFlowResponse, PreviewPolicyRequest, PreviewPolicyResponse, PublishConnectionIdentityRequest, PublishConnectionIdentityResponse, ReportConnectionRuntimeRequest, ReportConnectionRuntimeResponse, ReportEndpointDiscoveryRequest, ReportEndpointDiscoveryResponse, RevokeNodeRequest, RevokeNodeResponse, SaveConnectionsRequest, SaveMeshAccessRequest, SaveNetworkRequest, SavePolicyRequest, SaveTagsRequest, SetNodeCountryRequest, SetNodeTagsRequest, TagSnapshot } from "./types.gen.ts";
import { zAcknowledgeAgentConnectionsRequest, zAcknowledgeAgentConnectionsResponse, zAcknowledgeConfigurationRequest, zAcknowledgeConfigurationResponse, zAgentConnectionsSnapshot, zAgentDataplaneSnapshot, zAgentDnsSnapshot, zAgentNodeResponse, zAuthorizeDeviceRequest, zAuthorizeDeviceResponse, zBeginConnectionRuntimeRequest, zBeginConnectionRuntimeResponse, zConfigurationSnapshot, zConnectionRuntimesSnapshot, zConnectionsSnapshot, zCreateEnrollmentTokenRequest, zCreateEnrollmentTokenResponse, zEndpointDiscoveriesSnapshot, zEnrollAgentRequest, zExchangeDeviceRequest, zGetConnectionRuntimeEpochResponse, zHeartbeatRequest, zMeshAccessSnapshot, zNetworkSnapshot, zNodeList, zOverview, zPolicySnapshot, zPreviewFlowRequest, zPreviewFlowResponse, zPreviewPolicyRequest, zPreviewPolicyResponse, zPublishConnectionIdentityRequest, zPublishConnectionIdentityResponse, zReportConnectionRuntimeRequest, zReportConnectionRuntimeResponse, zReportEndpointDiscoveryRequest, zReportEndpointDiscoveryResponse, zRevokeNodeRequest, zRevokeNodeResponse, zSaveConnectionsRequest, zSaveMeshAccessRequest, zSaveNetworkRequest, zSavePolicyRequest, zSaveTagsRequest, zSetNodeCountryRequest, zSetNodeTagsRequest, zTagSnapshot } from "./zod.gen.ts";

export async function getMeshAccessOperation(baseUrl: string, token: string): Promise<MeshAccessSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/access", token }, zMeshAccessSnapshot);
}

export async function saveMeshAccessOperation(baseUrl: string, token: string, body: SaveMeshAccessRequest): Promise<MeshAccessSnapshot> {
  return request(baseUrl, { method: "POST", path: "/api/v1/access", token, body: zSaveMeshAccessRequest.parse(body) }, zMeshAccessSnapshot);
}

export async function publishConnectionIdentityOperation(baseUrl: string, token: string, body: PublishConnectionIdentityRequest): Promise<PublishConnectionIdentityResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/connection-identity", token, body: zPublishConnectionIdentityRequest.parse(body) }, zPublishConnectionIdentityResponse);
}

export async function getAgentConnectionsOperation(baseUrl: string, token: string): Promise<AgentConnectionsSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agent/connections", token }, zAgentConnectionsSnapshot);
}

export async function acknowledgeAgentConnectionsOperation(baseUrl: string, token: string, body: AcknowledgeAgentConnectionsRequest): Promise<AcknowledgeAgentConnectionsResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/connections/ack", token, body: zAcknowledgeAgentConnectionsRequest.parse(body) }, zAcknowledgeAgentConnectionsResponse);
}

export async function reportEndpointDiscoveryOperation(baseUrl: string, token: string, body: ReportEndpointDiscoveryRequest): Promise<ReportEndpointDiscoveryResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/connections/discovery", token, body: zReportEndpointDiscoveryRequest.parse(body) }, zReportEndpointDiscoveryResponse);
}

export async function getConnectionRuntimeEpochOperation(baseUrl: string, token: string): Promise<GetConnectionRuntimeEpochResponse> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agent/connections/runtime", token }, zGetConnectionRuntimeEpochResponse);
}

export async function reportConnectionRuntimeOperation(baseUrl: string, token: string, body: ReportConnectionRuntimeRequest): Promise<ReportConnectionRuntimeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/connections/runtime", token, body: zReportConnectionRuntimeRequest.parse(body) }, zReportConnectionRuntimeResponse);
}

export async function beginConnectionRuntimeOperation(baseUrl: string, token: string, body: BeginConnectionRuntimeRequest): Promise<BeginConnectionRuntimeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/connections/runtime/begin", token, body: zBeginConnectionRuntimeRequest.parse(body) }, zBeginConnectionRuntimeResponse);
}

export async function getAgentDataplaneOperation(baseUrl: string, token: string): Promise<AgentDataplaneSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agent/dataplane", token }, zAgentDataplaneSnapshot);
}

export async function getAgentDnsOperation(baseUrl: string, token: string): Promise<AgentDnsSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agent/dns", token }, zAgentDnsSnapshot);
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

export async function getEndpointDiscoveriesOperation(baseUrl: string, token: string): Promise<EndpointDiscoveriesSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/connections/discovery", token }, zEndpointDiscoveriesSnapshot);
}

export async function getConnectionRuntimesOperation(baseUrl: string, token: string): Promise<ConnectionRuntimesSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/connections/runtime", token }, zConnectionRuntimesSnapshot);
}

export async function authorizeDeviceOperation(baseUrl: string, token: string, body: AuthorizeDeviceRequest): Promise<AuthorizeDeviceResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/device-authorizations", token, body: zAuthorizeDeviceRequest.parse(body) }, zAuthorizeDeviceResponse);
}

export async function exchangeDeviceOperation(baseUrl: string, token: string, body: ExchangeDeviceRequest): Promise<AgentNodeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/devices/exchange", token, body: zExchangeDeviceRequest.parse(body) }, zAgentNodeResponse);
}

export async function createEnrollmentTokenOperation(baseUrl: string, token: string, body: CreateEnrollmentTokenRequest): Promise<CreateEnrollmentTokenResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/enrollment-tokens", token, body: zCreateEnrollmentTokenRequest.parse(body) }, zCreateEnrollmentTokenResponse);
}

export async function previewFlowOperation(baseUrl: string, token: string, body: PreviewFlowRequest): Promise<PreviewFlowResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/flows/preview", token, body: zPreviewFlowRequest.parse(body) }, zPreviewFlowResponse);
}

export async function getNetworkOperation(baseUrl: string, token: string): Promise<NetworkSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/network", token }, zNetworkSnapshot);
}

export async function saveNetworkOperation(baseUrl: string, token: string, body: SaveNetworkRequest): Promise<NetworkSnapshot> {
  return request(baseUrl, { method: "POST", path: "/api/v1/network", token, body: zSaveNetworkRequest.parse(body) }, zNetworkSnapshot);
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

export async function setNodeTagsOperation(baseUrl: string, token: string, body: SetNodeTagsRequest): Promise<TagSnapshot> {
  return request(baseUrl, { method: "POST", path: "/api/v1/nodes/tags", token, body: zSetNodeTagsRequest.parse(body) }, zTagSnapshot);
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

export async function getTagsOperation(baseUrl: string, token: string): Promise<TagSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/tags", token }, zTagSnapshot);
}

export async function saveTagsOperation(baseUrl: string, token: string, body: SaveTagsRequest): Promise<TagSnapshot> {
  return request(baseUrl, { method: "POST", path: "/api/v1/tags", token, body: zSaveTagsRequest.parse(body) }, zTagSnapshot);
}
