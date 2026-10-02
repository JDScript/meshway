// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import { request } from "../transport.ts";
import type { AcceptInvitationRequest, AccountInvitation, AccountInvitations, AccountMembers, AccountMembership, AccountPasswordReset, AccountPasswordResets, AccountReceipt, AccountSession, AcknowledgeAgentConnectionsRequest, AcknowledgeAgentConnectionsResponse, AcknowledgeConfigurationRequest, AcknowledgeConfigurationResponse, AgentConnectionsSnapshot, AgentDataplaneSnapshot, AgentDnsSnapshot, AgentManagedConnectionsSnapshot, AgentManagedDataplaneSnapshot, AgentNodeResponse, AuthorizeDeviceRequest, AuthorizeDeviceResponse, BeginConnectionRuntimeRequest, BeginConnectionRuntimeResponse, ChangePasswordRequest, ClaimNodeOwnershipRequest, ClaimOwnerRequest, ConfigurationSnapshot, ConnectionRuntimesSnapshot, ConnectionsSnapshot, CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse, CreateInvitationRequest, CreateInvitationResponse, CreateManagedEnrollmentTokenRequest, CreateNetworkRequest, CreatePasswordResetRequest, CreatePasswordResetResponse, EndpointDiscoveriesSnapshot, EnrollAgentRequest, ExchangeDeviceRequest, GetConnectionRuntimeEpochResponse, HeartbeatRequest, LatencyHistoryResponse, LoginRequest, LoginResponse, ManagedListenersSnapshot, MeshAccessSnapshot, NetworkList, NetworkSnapshot, NodeList, NodeOwnershipList, Overview, PolicySnapshot, PreviewFlowRequest, PreviewFlowResponse, PreviewPolicyRequest, PreviewPolicyResponse, PublishConnectionIdentityRequest, PublishConnectionIdentityResponse, PublishManagedTransportCapabilitiesRequest, PublishManagedTransportCapabilitiesResponse, QueryLatencyHistoryRequest, RedeemPasswordResetRequest, RegisterInvitationRequest, ReportConnectionRuntimeRequest, ReportConnectionRuntimeResponse, ReportEndpointDiscoveryRequest, ReportEndpointDiscoveryResponse, ReportLatencySamplesRequest, ReportLatencySamplesResponse, RevokeAccountSessionsRequest, RevokeInvitationRequest, RevokeNodeRequest, RevokeNodeResponse, RevokePasswordResetRequest, SaveConnectionsRequest, SaveManagedListenerRequest, SaveMembershipRequest, SaveMeshAccessRequest, SaveNetworkRequest, SavePolicyRequest, SaveTagsRequest, SetNodeCountryRequest, SetNodeTagsRequest, TagSnapshot } from "./types.gen.ts";
import { zAcceptInvitationRequest, zAccountInvitation, zAccountInvitations, zAccountMembers, zAccountMembership, zAccountPasswordReset, zAccountPasswordResets, zAccountReceipt, zAccountSession, zAcknowledgeAgentConnectionsRequest, zAcknowledgeAgentConnectionsResponse, zAcknowledgeConfigurationRequest, zAcknowledgeConfigurationResponse, zAgentConnectionsSnapshot, zAgentDataplaneSnapshot, zAgentDnsSnapshot, zAgentManagedConnectionsSnapshot, zAgentManagedDataplaneSnapshot, zAgentNodeResponse, zAuthorizeDeviceRequest, zAuthorizeDeviceResponse, zBeginConnectionRuntimeRequest, zBeginConnectionRuntimeResponse, zChangePasswordRequest, zClaimNodeOwnershipRequest, zClaimOwnerRequest, zConfigurationSnapshot, zConnectionRuntimesSnapshot, zConnectionsSnapshot, zCreateEnrollmentTokenRequest, zCreateEnrollmentTokenResponse, zCreateInvitationRequest, zCreateInvitationResponse, zCreateManagedEnrollmentTokenRequest, zCreateNetworkRequest, zCreatePasswordResetRequest, zCreatePasswordResetResponse, zEndpointDiscoveriesSnapshot, zEnrollAgentRequest, zExchangeDeviceRequest, zGetConnectionRuntimeEpochResponse, zHeartbeatRequest, zLatencyHistoryResponse, zLoginRequest, zLoginResponse, zManagedListenersSnapshot, zMeshAccessSnapshot, zNetworkList, zNetworkSnapshot, zNodeList, zNodeOwnershipList, zOverview, zPolicySnapshot, zPreviewFlowRequest, zPreviewFlowResponse, zPreviewPolicyRequest, zPreviewPolicyResponse, zPublishConnectionIdentityRequest, zPublishConnectionIdentityResponse, zPublishManagedTransportCapabilitiesRequest, zPublishManagedTransportCapabilitiesResponse, zQueryLatencyHistoryRequest, zRedeemPasswordResetRequest, zRegisterInvitationRequest, zReportConnectionRuntimeRequest, zReportConnectionRuntimeResponse, zReportEndpointDiscoveryRequest, zReportEndpointDiscoveryResponse, zReportLatencySamplesRequest, zReportLatencySamplesResponse, zRevokeAccountSessionsRequest, zRevokeInvitationRequest, zRevokeNodeRequest, zRevokeNodeResponse, zRevokePasswordResetRequest, zSaveConnectionsRequest, zSaveManagedListenerRequest, zSaveMembershipRequest, zSaveMeshAccessRequest, zSaveNetworkRequest, zSavePolicyRequest, zSaveTagsRequest, zSetNodeCountryRequest, zSetNodeTagsRequest, zTagSnapshot } from "./zod.gen.ts";

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

export async function reportLatencySamplesOperation(baseUrl: string, token: string, body: ReportLatencySamplesRequest): Promise<ReportLatencySamplesResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/latency/samples", token, body: zReportLatencySamplesRequest.parse(body) }, zReportLatencySamplesResponse);
}

export async function getAgentManagedConnectionsOperation(baseUrl: string, token: string): Promise<AgentManagedConnectionsSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agent/managed-connections", token }, zAgentManagedConnectionsSnapshot);
}

export async function getAgentManagedDataplaneOperation(baseUrl: string, token: string): Promise<AgentManagedDataplaneSnapshot> {
  return request(baseUrl, { method: "GET", path: "/api/v1/agent/managed-dataplane", token }, zAgentManagedDataplaneSnapshot);
}

export async function publishManagedTransportCapabilitiesOperation(baseUrl: string, token: string, body: PublishManagedTransportCapabilitiesRequest): Promise<PublishManagedTransportCapabilitiesResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/agent/managed-transport-capabilities", token, body: zPublishManagedTransportCapabilitiesRequest.parse(body) }, zPublishManagedTransportCapabilitiesResponse);
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

export async function acceptAccountInvitationOperation(baseUrl: string, token: string, body: AcceptInvitationRequest): Promise<AccountMembership> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/invitations/accept", token, body: zAcceptInvitationRequest.parse(body) }, zAccountMembership);
}

export async function registerInvitedAccountOperation(baseUrl: string, body: RegisterInvitationRequest): Promise<AccountMembership> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/invitations/register", body: zRegisterInvitationRequest.parse(body) }, zAccountMembership);
}

export async function loginAccountOperation(baseUrl: string, body: LoginRequest): Promise<LoginResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/login", body: zLoginRequest.parse(body) }, zLoginResponse);
}

export async function logoutAccountOperation(baseUrl: string, token: string): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/logout", token }, zAccountReceipt);
}

export async function listAccountPasswordResetsOperation(baseUrl: string, token: string): Promise<AccountPasswordResets> {
  return request(baseUrl, { method: "GET", path: "/api/v1/auth/password-resets", token }, zAccountPasswordResets);
}

export async function createAccountPasswordResetOperation(baseUrl: string, token: string, body: CreatePasswordResetRequest): Promise<CreatePasswordResetResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/password-resets", token, body: zCreatePasswordResetRequest.parse(body) }, zCreatePasswordResetResponse);
}

export async function redeemAccountPasswordResetOperation(baseUrl: string, body: RedeemPasswordResetRequest): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/password-resets/redeem", body: zRedeemPasswordResetRequest.parse(body) }, zAccountReceipt);
}

export async function revokeAccountPasswordResetOperation(baseUrl: string, token: string, body: RevokePasswordResetRequest): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/password-resets/revoke", token, body: zRevokePasswordResetRequest.parse(body) }, zAccountReceipt);
}

export async function getAccountPasswordResetOperation(baseUrl: string, token: string, reset_id: string): Promise<AccountPasswordReset> {
  return request(baseUrl, { method: "GET", path: `/api/v1/auth/password-resets/${encodeURIComponent(reset_id)}`, token }, zAccountPasswordReset);
}

export async function getAccountSessionOperation(baseUrl: string, token: string): Promise<AccountSession> {
  return request(baseUrl, { method: "GET", path: "/api/v1/auth/session", token }, zAccountSession);
}

export async function revokeAccountSessionsOperation(baseUrl: string, token: string, body: RevokeAccountSessionsRequest): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: "/api/v1/auth/sessions/revoke", token, body: zRevokeAccountSessionsRequest.parse(body) }, zAccountReceipt);
}

export async function exchangeDeviceOperation(baseUrl: string, token: string, body: ExchangeDeviceRequest): Promise<AgentNodeResponse> {
  return request(baseUrl, { method: "POST", path: "/api/v1/devices/exchange", token, body: zExchangeDeviceRequest.parse(body) }, zAgentNodeResponse);
}

export async function changeAccountPasswordOperation(baseUrl: string, token: string, body: ChangePasswordRequest): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: "/api/v1/me/password", token, body: zChangePasswordRequest.parse(body) }, zAccountReceipt);
}

export async function listNetworksOperation(baseUrl: string, token: string): Promise<NetworkList> {
  return request(baseUrl, { method: "GET", path: "/api/v1/networks", token }, zNetworkList);
}

export async function createNetworkOperation(baseUrl: string, token: string, body: CreateNetworkRequest): Promise<NetworkSnapshot> {
  return request(baseUrl, { method: "POST", path: "/api/v1/networks", token, body: zCreateNetworkRequest.parse(body) }, zNetworkSnapshot);
}

export async function getMeshAccessOperation(baseUrl: string, token: string, network_id: string): Promise<MeshAccessSnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/access`, token }, zMeshAccessSnapshot);
}

export async function saveMeshAccessOperation(baseUrl: string, token: string, network_id: string, body: SaveMeshAccessRequest): Promise<MeshAccessSnapshot> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/access`, token, body: zSaveMeshAccessRequest.parse(body) }, zMeshAccessSnapshot);
}

export async function getConnectionsOperation(baseUrl: string, token: string, network_id: string): Promise<ConnectionsSnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/connections`, token }, zConnectionsSnapshot);
}

export async function saveConnectionsOperation(baseUrl: string, token: string, network_id: string, body: SaveConnectionsRequest): Promise<ConnectionsSnapshot> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/connections`, token, body: zSaveConnectionsRequest.parse(body) }, zConnectionsSnapshot);
}

export async function getEndpointDiscoveriesOperation(baseUrl: string, token: string, network_id: string): Promise<EndpointDiscoveriesSnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/connections/discovery`, token }, zEndpointDiscoveriesSnapshot);
}

export async function getConnectionRuntimesOperation(baseUrl: string, token: string, network_id: string): Promise<ConnectionRuntimesSnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/connections/runtime`, token }, zConnectionRuntimesSnapshot);
}

export async function authorizeDeviceOperation(baseUrl: string, token: string, network_id: string, body: AuthorizeDeviceRequest): Promise<AuthorizeDeviceResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/device-authorizations`, token, body: zAuthorizeDeviceRequest.parse(body) }, zAuthorizeDeviceResponse);
}

export async function createEnrollmentTokenOperation(baseUrl: string, token: string, network_id: string, body: CreateEnrollmentTokenRequest): Promise<CreateEnrollmentTokenResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/enrollment-tokens`, token, body: zCreateEnrollmentTokenRequest.parse(body) }, zCreateEnrollmentTokenResponse);
}

export async function previewFlowOperation(baseUrl: string, token: string, network_id: string, body: PreviewFlowRequest): Promise<PreviewFlowResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/flows/preview`, token, body: zPreviewFlowRequest.parse(body) }, zPreviewFlowResponse);
}

export async function listAccountInvitationsOperation(baseUrl: string, token: string, network_id: string): Promise<AccountInvitations> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/invitations`, token }, zAccountInvitations);
}

export async function createAccountInvitationOperation(baseUrl: string, token: string, network_id: string, body: CreateInvitationRequest): Promise<CreateInvitationResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/invitations`, token, body: zCreateInvitationRequest.parse(body) }, zCreateInvitationResponse);
}

export async function revokeAccountInvitationOperation(baseUrl: string, token: string, network_id: string, body: RevokeInvitationRequest): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/invitations/revoke`, token, body: zRevokeInvitationRequest.parse(body) }, zAccountReceipt);
}

export async function getAccountInvitationOperation(baseUrl: string, token: string, network_id: string, invitation_id: string): Promise<AccountInvitation> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/invitations/${encodeURIComponent(invitation_id)}`, token }, zAccountInvitation);
}

export async function queryLatencyHistoryOperation(baseUrl: string, token: string, network_id: string, body: QueryLatencyHistoryRequest): Promise<LatencyHistoryResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/latency/history/query`, token, body: zQueryLatencyHistoryRequest.parse(body) }, zLatencyHistoryResponse);
}

export async function createManagedEnrollmentTokenOperation(baseUrl: string, token: string, network_id: string, body: CreateManagedEnrollmentTokenRequest): Promise<CreateEnrollmentTokenResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/managed-enrollment-tokens`, token, body: zCreateManagedEnrollmentTokenRequest.parse(body) }, zCreateEnrollmentTokenResponse);
}

export async function getManagedListenersOperation(baseUrl: string, token: string, network_id: string): Promise<ManagedListenersSnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/managed-listeners`, token }, zManagedListenersSnapshot);
}

export async function saveManagedListenerOperation(baseUrl: string, token: string, network_id: string, body: SaveManagedListenerRequest): Promise<ManagedListenersSnapshot> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/managed-listeners`, token, body: zSaveManagedListenerRequest.parse(body) }, zManagedListenersSnapshot);
}

export async function listAccountMembersOperation(baseUrl: string, token: string, network_id: string): Promise<AccountMembers> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/members`, token }, zAccountMembers);
}

export async function leaveAccountNetworkOperation(baseUrl: string, token: string, network_id: string): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/members/leave`, token }, zAccountReceipt);
}

export async function saveAccountMemberOperation(baseUrl: string, token: string, network_id: string, body: SaveMembershipRequest): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/members/save`, token, body: zSaveMembershipRequest.parse(body) }, zAccountReceipt);
}

export async function getNetworkOperation(baseUrl: string, token: string, network_id: string): Promise<NetworkSnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/network`, token }, zNetworkSnapshot);
}

export async function saveNetworkOperation(baseUrl: string, token: string, network_id: string, body: SaveNetworkRequest): Promise<NetworkSnapshot> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/network`, token, body: zSaveNetworkRequest.parse(body) }, zNetworkSnapshot);
}

export async function listNodeOwnershipOperation(baseUrl: string, token: string, network_id: string): Promise<NodeOwnershipList> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/node-ownership`, token }, zNodeOwnershipList);
}

export async function claimNodeOwnershipOperation(baseUrl: string, token: string, network_id: string, body: ClaimNodeOwnershipRequest): Promise<AccountReceipt> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/node-ownership/claim`, token, body: zClaimNodeOwnershipRequest.parse(body) }, zAccountReceipt);
}

export async function listNodesOperation(baseUrl: string, token: string, network_id: string): Promise<NodeList> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/nodes`, token }, zNodeList);
}

export async function setNodeCountryOperation(baseUrl: string, token: string, network_id: string, body: SetNodeCountryRequest): Promise<RevokeNodeResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/nodes/country`, token, body: zSetNodeCountryRequest.parse(body) }, zRevokeNodeResponse);
}

export async function revokeNodeOperation(baseUrl: string, token: string, network_id: string, body: RevokeNodeRequest): Promise<RevokeNodeResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/nodes/revoke`, token, body: zRevokeNodeRequest.parse(body) }, zRevokeNodeResponse);
}

export async function setNodeTagsOperation(baseUrl: string, token: string, network_id: string, body: SetNodeTagsRequest): Promise<TagSnapshot> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/nodes/tags`, token, body: zSetNodeTagsRequest.parse(body) }, zTagSnapshot);
}

export async function getOverviewOperation(baseUrl: string, token: string, network_id: string): Promise<Overview> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/overview`, token }, zOverview);
}

export async function claimFirstOwnerOperation(baseUrl: string, token: string, network_id: string, body: ClaimOwnerRequest): Promise<AccountMembership> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/owner-claim`, token, body: zClaimOwnerRequest.parse(body) }, zAccountMembership);
}

export async function getPolicyOperation(baseUrl: string, token: string, network_id: string): Promise<PolicySnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/policy`, token }, zPolicySnapshot);
}

export async function previewPolicyOperation(baseUrl: string, token: string, network_id: string, body: PreviewPolicyRequest): Promise<PreviewPolicyResponse> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/policy/preview`, token, body: zPreviewPolicyRequest.parse(body) }, zPreviewPolicyResponse);
}

export async function savePolicyOperation(baseUrl: string, token: string, network_id: string, body: SavePolicyRequest): Promise<PolicySnapshot> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/policy/save`, token, body: zSavePolicyRequest.parse(body) }, zPolicySnapshot);
}

export async function getTagsOperation(baseUrl: string, token: string, network_id: string): Promise<TagSnapshot> {
  return request(baseUrl, { method: "GET", path: `/api/v1/networks/${encodeURIComponent(network_id)}/tags`, token }, zTagSnapshot);
}

export async function saveTagsOperation(baseUrl: string, token: string, network_id: string, body: SaveTagsRequest): Promise<TagSnapshot> {
  return request(baseUrl, { method: "POST", path: `/api/v1/networks/${encodeURIComponent(network_id)}/tags`, token, body: zSaveTagsRequest.parse(body) }, zTagSnapshot);
}
