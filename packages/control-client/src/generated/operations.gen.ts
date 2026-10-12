// Generated from Controller OpenAPI. Do not edit.

import type * as z from "zod";
import { ControlClient, type AuthenticatedRequestOptions, type AuthenticatedStreamOptions, type RequestOptions } from "../transport.ts";
import { zAcceptUdpTraversalPath, zAcceptUdpTraversalResponse, zAccountResponse, zAccountSessionsResponse, zAcquireInstanceRequest, zActivityListResponse, zActivityResponse, zApplyConfigurationPath, zApproveDeviceAuthorizationPath, zApproveEnrollmentRequest, zAuthOptionsResponse, zAuthSessionResponse, zCancelEnrollmentRequestPath, zCancelNodeObservationPath, zCancelObservationPath, zCancelUdpTraversalPath, zCancelUdpTraversalResponse, zChangeEmailRequest, zChangeMemberRolePath, zChangeMemberRoleRequest, zChangePasswordRequest, zCompareConfigurationPath, zCompareConfigurationRequest, zConfigurationAppliedResponse, zConfigurationApplyProgress, zConfigurationComparison, zConfigurationPreviewResponse, zConfigurationRequest, zConfigurationResponse, zConfigurationRevisionList, zConfigurationSchemaResponse, zConfigurationSourceResponse, zCreateDelegationRequest, zCreateEnrollmentKeyPath, zCreateEnrollmentKeyRequest, zCreateEnrollmentRequest, zCreateJoinLinkPath, zCreateJoinLinkRequest, zCreateNetworkRequest, zCreateNodeObservation, zCreateObservation, zCreateObservationPath, zDelegatedSessionResponse, zDelegationCreated, zDeleteNodePath, zDeleteNodeRequest, zDenyDeviceAuthorizationPath, zDeviceAuthorizationView, zDnsAssociationAccepted, zDnsAssociationReport, zEnrollmentDeviceProof, zEnrollmentKeyResponse, zEnrollmentKeysResponse, zEnrollmentPollResponse, zEnrollmentRequestView, zEnrollmentResponse, zErrorResponse, zEvaluateConfigurationPath, zEvaluateConfigurationRequest, zEvaluationResponse, zExchangeDelegationRequest, zGetConfigurationPath, zGetConfigurationProgressPath, zGetConfigurationRevisionPath, zGetConfigurationSourcePath, zGetConfigurationSourceQuery, zGetDeviceAuthorizationPath, zGetLeaveImpactPath, zGetMemberRemovalImpactPath, zGetMemberRoleChangeImpactPath, zGetNetworkActivityPath, zGetNetworkPath, zGetNetworkStatePath, zGetNodeObservationPath, zGetNodeObservationQuery, zGetNodePath, zGetNodeSettingsPath, zGetNodeStatePath, zGetObservationPath, zGetRuleSetPath, zGetRuleSetReportPath, zHistoryAccepted, zHistoryPage, zHistoryQuery, zHistoryReport, zInstanceLease, zInstanceRequest, zJoinLinkCreated, zJoinLinkCredentialRequest, zJoinLinkInspection, zJoinLinkListResponse, zJoinLinkRedeemed, zKeyEnrollmentRequest, zLeaveNetworkPath, zListAccountActivityQuery, zListAccountSessionsQuery, zListConfigurationRevisionsPath, zListConfigurationRevisionsQuery, zListEnrollmentKeysPath, zListEnrollmentKeysQuery, zListJoinLinksPath, zListJoinLinksQuery, zListNetworkActivityPath, zListNetworkActivityQuery, zListNetworkMembersPath, zListNetworkMembersQuery, zListNetworksQuery, zListNodesPath, zListObservationsPath, zListRuleSetsPath, zLoginRequest, zMemberMutationResponse, zMemberRemovedResponse, zNetworkListResponse, zNetworkMembersResponse, zNetworkResponse, zNodeChallenge, zNodeInventoryResponse, zNodeInventoryView, zNodeKeyMetadata, zNodeLeaveRequest, zNodeLeaveResponse, zNodeObservationCancelled, zNodeObservationResponse, zNodeRotationRequest, zNodeSessionRequest, zNodeSessionResponse, zNodeSettingsResponse, zObservationList, zObservationResult, zObservationTask, zPasswordResetInspectRequest, zPasswordResetInspection, zPasswordResetRedeemRequest, zPrepareUdpTraversalRequest, zPrepareUdpTraversalResponse, zQueryEnrollmentRequestPath, zQueryObservationHistoryPath, zQueryUdpTraversalRequest, zQueryUdpTraversalResponse, zReceiptRequest, zRedeemEnrollmentRequest, zRedeemEnrollmentRequestPath, zRefreshRuleSetPath, zRefreshRuleSetRequest, zRegisterRequest, zRemovalImpactResponse, zRemoveMemberPath, zRemoveMemberRequest, zRenameNetworkPath, zRenameNetworkRequest, zRenewNodeObservation, zRenewNodeObservationPath, zRenewObservationPath, zReportTaskResultPath, zRevokeAccountSessionPath, zRevokeEnrollmentKeyPath, zRevokeJoinLinkPath, zRevokeNodeKeyPath, zRevokeNodeKeyRequest, zRoleChangeImpactResponse, zRoutingSelectionRequest, zRoutingSelectionResponse, zRuleSetList, zRuleSetReport, zRuleSetView, zSessionIssuedResponse, zSignedNodeChallengeRequest, zStateAccepted, zStateQuery, zStateReport, zStateView, zSyncRequest, zSyncResponse, zTransportCatalogResponse, zUdpTraversalAttemptRequest, zUserEvent, zValidateConfigurationPath, zWatchNetworkPath } from "./zod.gen.ts";

export function acceptUdpTraversal(client: ControlClient, path: z.output<typeof zAcceptUdpTraversalPath>, body: z.output<typeof zUdpTraversalAttemptRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/connectivity/udp/attempts/{attemptId}/accept", authenticated: true, successStatus: 200, success: zAcceptUdpTraversalResponse, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zAcceptUdpTraversalPath, value: path }, body: { schema: zUdpTraversalAttemptRequest, value: body } }, options);
}

export function acquireNodeInstance(client: ControlClient, body: z.output<typeof zAcquireInstanceRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/instances", authenticated: true, successStatus: 200, success: zInstanceLease, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zAcquireInstanceRequest, value: body } }, options);
}

export function applyConfiguration(client: ControlClient, path: z.output<typeof zApplyConfigurationPath>, body: z.output<typeof zConfigurationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/configuration", authenticated: true, successStatus: 200, success: zConfigurationAppliedResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zApplyConfigurationPath, value: path }, body: { schema: zConfigurationRequest, value: body } }, options);
}

export function approveDeviceAuthorization(client: ControlClient, path: z.output<typeof zApproveDeviceAuthorizationPath>, body: z.output<typeof zApproveEnrollmentRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/device-authorizations/{enrollmentRequestId}/approve", authenticated: true, successStatus: 200, success: zDeviceAuthorizationView, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zApproveDeviceAuthorizationPath, value: path }, body: { schema: zApproveEnrollmentRequest, value: body } }, options);
}

export function cancelEnrollmentRequest(client: ControlClient, path: z.output<typeof zCancelEnrollmentRequestPath>, body: z.output<typeof zEnrollmentDeviceProof>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/enrollment-requests/{enrollmentRequestId}/cancel", authenticated: false, successStatus: 204, success: undefined, errors: [400, 401, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zCancelEnrollmentRequestPath, value: path }, body: { schema: zEnrollmentDeviceProof, value: body } }, options);
}

export function cancelNodeObservation(client: ControlClient, path: z.output<typeof zCancelNodeObservationPath>, body: z.output<typeof zInstanceRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/observations/{subscriptionId}/cancel", authenticated: true, successStatus: 200, success: zNodeObservationCancelled, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zCancelNodeObservationPath, value: path }, body: { schema: zInstanceRequest, value: body } }, options);
}

export function cancelObservation(client: ControlClient, path: z.output<typeof zCancelObservationPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/observations/{subscriptionId}/cancel", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 404, 409, 422, 503], problem: zErrorResponse, pathParameters: { schema: zCancelObservationPath, value: path } }, options);
}

export function cancelUdpTraversal(client: ControlClient, path: z.output<typeof zCancelUdpTraversalPath>, body: z.output<typeof zUdpTraversalAttemptRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/connectivity/udp/attempts/{attemptId}/cancel", authenticated: true, successStatus: 200, success: zCancelUdpTraversalResponse, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zCancelUdpTraversalPath, value: path }, body: { schema: zUdpTraversalAttemptRequest, value: body } }, options);
}

export function changeAccountEmail(client: ControlClient, body: z.output<typeof zChangeEmailRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/email", authenticated: true, successStatus: 200, success: zAccountResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zChangeEmailRequest, value: body } }, options);
}

export function changeAccountPassword(client: ControlClient, body: z.output<typeof zChangePasswordRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/password", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zChangePasswordRequest, value: body } }, options);
}

export function changeMemberRole(client: ControlClient, path: z.output<typeof zChangeMemberRolePath>, body: z.output<typeof zChangeMemberRoleRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/members/{membershipId}/role", authenticated: true, successStatus: 200, success: zMemberMutationResponse, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zChangeMemberRolePath, value: path }, body: { schema: zChangeMemberRoleRequest, value: body } }, options);
}

export function compareConfiguration(client: ControlClient, path: z.output<typeof zCompareConfigurationPath>, body: z.output<typeof zCompareConfigurationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/configuration/compare", authenticated: true, successStatus: 200, success: zConfigurationComparison, errors: [400, 401, 403, 404, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zCompareConfigurationPath, value: path }, body: { schema: zCompareConfigurationRequest, value: body } }, options);
}

export function createDelegation(client: ControlClient, body: z.output<typeof zCreateDelegationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/auth/delegations", authenticated: true, successStatus: 201, success: zDelegationCreated, errors: [400, 401, 403, 413, 415, 503], problem: zErrorResponse, body: { schema: zCreateDelegationRequest, value: body } }, options);
}

export function createEnrollmentKey(client: ControlClient, path: z.output<typeof zCreateEnrollmentKeyPath>, body: z.output<typeof zCreateEnrollmentKeyRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/enrollment-keys", authenticated: true, successStatus: 200, success: zEnrollmentKeyResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zCreateEnrollmentKeyPath, value: path }, body: { schema: zCreateEnrollmentKeyRequest, value: body } }, options);
}

export function createEnrollmentRequest(client: ControlClient, body: z.output<typeof zCreateEnrollmentRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/enrollment-requests", authenticated: false, successStatus: 200, success: zEnrollmentRequestView, errors: [400, 401, 404, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zCreateEnrollmentRequest, value: body } }, options);
}

export function createJoinLink(client: ControlClient, path: z.output<typeof zCreateJoinLinkPath>, body: z.output<typeof zCreateJoinLinkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/join-links", authenticated: true, successStatus: 201, success: zJoinLinkCreated, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zCreateJoinLinkPath, value: path }, body: { schema: zCreateJoinLinkRequest, value: body } }, options);
}

export function createNetwork(client: ControlClient, body: z.output<typeof zCreateNetworkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks", authenticated: true, successStatus: 201, success: zNetworkResponse, errors: [400, 401, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zCreateNetworkRequest, value: body } }, options);
}

export function createNodeChallenge(client: ControlClient, body: z.output<typeof zSignedNodeChallengeRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/auth/challenges", authenticated: false, successStatus: 200, success: zNodeChallenge, errors: [400, 401, 403, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zSignedNodeChallengeRequest, value: body } }, options);
}

export function createNodeObservation(client: ControlClient, body: z.output<typeof zCreateNodeObservation>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/observations", authenticated: true, successStatus: 201, success: zNodeObservationResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 429, 503], problem: zErrorResponse, body: { schema: zCreateNodeObservation, value: body } }, options);
}

export function createNodeSession(client: ControlClient, body: z.output<typeof zNodeSessionRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/auth/session", authenticated: false, successStatus: 200, success: zNodeSessionResponse, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, body: { schema: zNodeSessionRequest, value: body } }, options);
}

export function createObservation(client: ControlClient, path: z.output<typeof zCreateObservationPath>, body: z.output<typeof zCreateObservation>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/observations", authenticated: true, successStatus: 201, success: zObservationTask, errors: [400, 401, 403, 404, 409, 413, 415, 422, 429, 503], problem: zErrorResponse, pathParameters: { schema: zCreateObservationPath, value: path }, body: { schema: zCreateObservation, value: body } }, options);
}

export function deleteNode(client: ControlClient, path: z.output<typeof zDeleteNodePath>, body: z.output<typeof zDeleteNodeRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "DELETE", path: "/api/v1/networks/{networkId}/nodes/{nodeId}", authenticated: true, successStatus: 200, success: zConfigurationAppliedResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zDeleteNodePath, value: path }, body: { schema: zDeleteNodeRequest, value: body } }, options);
}

export function denyDeviceAuthorization(client: ControlClient, path: z.output<typeof zDenyDeviceAuthorizationPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/device-authorizations/{enrollmentRequestId}/deny", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 404, 409, 503], problem: zErrorResponse, pathParameters: { schema: zDenyDeviceAuthorizationPath, value: path } }, options);
}

export function enrollNodeWithKey(client: ControlClient, body: z.output<typeof zKeyEnrollmentRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/enrollments", authenticated: false, successStatus: 200, success: zEnrollmentResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zKeyEnrollmentRequest, value: body } }, options);
}

export function evaluateConfiguration(client: ControlClient, path: z.output<typeof zEvaluateConfigurationPath>, body: z.output<typeof zEvaluateConfigurationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/configuration/evaluate", authenticated: true, successStatus: 200, success: zEvaluationResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 429, 503], problem: zErrorResponse, pathParameters: { schema: zEvaluateConfigurationPath, value: path }, body: { schema: zEvaluateConfigurationRequest, value: body } }, options);
}

export function exchangeDelegation(client: ControlClient, body: z.output<typeof zExchangeDelegationRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/delegations/token", authenticated: false, successStatus: 200, success: zDelegatedSessionResponse, errors: [400, 413, 415, 503], problem: zErrorResponse, body: { schema: zExchangeDelegationRequest, value: body } }, options);
}

export function getAccount(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/account", authenticated: true, successStatus: 200, success: zAccountResponse, errors: [401, 503], problem: zErrorResponse }, options);
}

export function getAuthOptions(client: ControlClient, options: RequestOptions = {}) {
  return client.request({ method: "GET", path: "/api/v1/auth/options", authenticated: false, successStatus: 200, success: zAuthOptionsResponse, errors: [503], problem: zErrorResponse }, options);
}

export function getAuthSession(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/auth/session", authenticated: true, successStatus: 200, success: zAuthSessionResponse, errors: [401, 503], problem: zErrorResponse }, options);
}

export function getConfiguration(client: ControlClient, path: z.output<typeof zGetConfigurationPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/configuration", authenticated: true, successStatus: 200, success: zConfigurationResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetConfigurationPath, value: path } }, options);
}

export function getConfigurationProgress(client: ControlClient, path: z.output<typeof zGetConfigurationProgressPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/configuration/revisions/{revision}/progress", authenticated: true, successStatus: 200, success: zConfigurationApplyProgress, errors: [400, 401, 403, 404, 409, 429, 503], problem: zErrorResponse, pathParameters: { schema: zGetConfigurationProgressPath, value: path } }, options);
}

export function getConfigurationRevision(client: ControlClient, path: z.output<typeof zGetConfigurationRevisionPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/configuration/revisions/{revision}", authenticated: true, successStatus: 200, success: zConfigurationResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetConfigurationRevisionPath, value: path } }, options);
}

export function getConfigurationSchema(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/configuration/schema", authenticated: true, successStatus: 200, success: zConfigurationSchemaResponse, errors: [401, 503], problem: zErrorResponse }, options);
}

export function getConfigurationSource(client: ControlClient, path: z.output<typeof zGetConfigurationSourcePath>, query: z.output<typeof zGetConfigurationSourceQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/configuration/source", authenticated: true, successStatus: 200, success: zConfigurationSourceResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetConfigurationSourcePath, value: path }, queryParameters: { schema: zGetConfigurationSourceQuery, value: query } }, options);
}

export function getDeviceAuthorization(client: ControlClient, path: z.output<typeof zGetDeviceAuthorizationPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/device-authorizations/{enrollmentRequestId}", authenticated: true, successStatus: 200, success: zDeviceAuthorizationView, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetDeviceAuthorizationPath, value: path } }, options);
}

export function getLeaveImpact(client: ControlClient, path: z.output<typeof zGetLeaveImpactPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/leave-impact", authenticated: true, successStatus: 200, success: zRemovalImpactResponse, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetLeaveImpactPath, value: path } }, options);
}

export function getMemberRemovalImpact(client: ControlClient, path: z.output<typeof zGetMemberRemovalImpactPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/members/{membershipId}/removal-impact", authenticated: true, successStatus: 200, success: zRemovalImpactResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetMemberRemovalImpactPath, value: path } }, options);
}

export function getMemberRoleChangeImpact(client: ControlClient, path: z.output<typeof zGetMemberRoleChangeImpactPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/members/{membershipId}/role-change-impact/{role}", authenticated: true, successStatus: 200, success: zRoleChangeImpactResponse, errors: [400, 401, 403, 404, 409, 503], problem: zErrorResponse, pathParameters: { schema: zGetMemberRoleChangeImpactPath, value: path } }, options);
}

export function getNetwork(client: ControlClient, path: z.output<typeof zGetNetworkPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}", authenticated: true, successStatus: 200, success: zNetworkResponse, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetNetworkPath, value: path } }, options);
}

export function getNetworkActivity(client: ControlClient, path: z.output<typeof zGetNetworkActivityPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/activity/{activityId}", authenticated: true, successStatus: 200, success: zActivityResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetNetworkActivityPath, value: path } }, options);
}

export function getNetworkState(client: ControlClient, path: z.output<typeof zGetNetworkStatePath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/state", authenticated: true, successStatus: 200, success: zStateView, errors: [400, 401, 403, 404, 409, 429, 503], problem: zErrorResponse, pathParameters: { schema: zGetNetworkStatePath, value: path } }, options);
}

export function getNode(client: ControlClient, path: z.output<typeof zGetNodePath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/nodes/{nodeId}", authenticated: true, successStatus: 200, success: zNodeInventoryView, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetNodePath, value: path } }, options);
}

export function getNodeObservation(client: ControlClient, path: z.output<typeof zGetNodeObservationPath>, query: z.output<typeof zGetNodeObservationQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/agent-api/v1/observations/{subscriptionId}", authenticated: true, successStatus: 200, success: zNodeObservationResponse, errors: [400, 401, 403, 404, 409, 422, 503], problem: zErrorResponse, pathParameters: { schema: zGetNodeObservationPath, value: path }, queryParameters: { schema: zGetNodeObservationQuery, value: query } }, options);
}

export function getNodeSettings(client: ControlClient, path: z.output<typeof zGetNodeSettingsPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/nodes/{nodeId}/settings", authenticated: true, successStatus: 200, success: zNodeSettingsResponse, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetNodeSettingsPath, value: path } }, options);
}

export function getNodeState(client: ControlClient, path: z.output<typeof zGetNodeStatePath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/nodes/{nodeId}/state", authenticated: true, successStatus: 200, success: zStateView, errors: [400, 401, 403, 404, 409, 429, 503], problem: zErrorResponse, pathParameters: { schema: zGetNodeStatePath, value: path } }, options);
}

export function getNodeTasks(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/agent-api/v1/tasks", authenticated: true, successStatus: 200, success: zObservationList, errors: [400, 401, 403, 404, 409, 422, 503], problem: zErrorResponse }, options);
}

export function getObservation(client: ControlClient, path: z.output<typeof zGetObservationPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/observations/{subscriptionId}", authenticated: true, successStatus: 200, success: zObservationList, errors: [400, 401, 403, 404, 409, 422, 503], problem: zErrorResponse, pathParameters: { schema: zGetObservationPath, value: path } }, options);
}

export function getRuleSet(client: ControlClient, path: z.output<typeof zGetRuleSetPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}", authenticated: true, successStatus: 200, success: zRuleSetList, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetRuleSetPath, value: path } }, options);
}

export function getRuleSetReport(client: ControlClient, path: z.output<typeof zGetRuleSetReportPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}/report", authenticated: true, successStatus: 200, success: zRuleSetReport, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetRuleSetReportPath, value: path } }, options);
}

export function getTransportCatalog(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/transport-catalog", authenticated: true, successStatus: 200, success: zTransportCatalogResponse, errors: [401, 503], problem: zErrorResponse }, options);
}

export function inspectJoinLink(client: ControlClient, body: z.output<typeof zJoinLinkCredentialRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/join-links/inspect", authenticated: true, successStatus: 200, success: zJoinLinkInspection, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, body: { schema: zJoinLinkCredentialRequest, value: body } }, options);
}

export function inspectPasswordReset(client: ControlClient, body: z.output<typeof zPasswordResetInspectRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/password-reset/inspect", authenticated: false, successStatus: 200, success: zPasswordResetInspection, errors: [400, 413, 415, 503], problem: zErrorResponse, body: { schema: zPasswordResetInspectRequest, value: body } }, options);
}

export function leaveNetwork(client: ControlClient, path: z.output<typeof zLeaveNetworkPath>, body: z.output<typeof zRemoveMemberRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/leave", authenticated: true, successStatus: 200, success: zMemberRemovedResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zLeaveNetworkPath, value: path }, body: { schema: zRemoveMemberRequest, value: body } }, options);
}

export function leaveNode(client: ControlClient, body: z.output<typeof zNodeLeaveRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/leave", authenticated: true, successStatus: 200, success: zNodeLeaveResponse, errors: [400, 401, 403, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zNodeLeaveRequest, value: body } }, options);
}

export function listAccountActivity(client: ControlClient, query: z.output<typeof zListAccountActivityQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/account/activity", authenticated: true, successStatus: 200, success: zActivityListResponse, errors: [400, 401, 503], problem: zErrorResponse, queryParameters: { schema: zListAccountActivityQuery, value: query } }, options);
}

export function listAccountSessions(client: ControlClient, query: z.output<typeof zListAccountSessionsQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/account/sessions", authenticated: true, successStatus: 200, success: zAccountSessionsResponse, errors: [400, 401, 503], problem: zErrorResponse, queryParameters: { schema: zListAccountSessionsQuery, value: query } }, options);
}

export function listConfigurationRevisions(client: ControlClient, path: z.output<typeof zListConfigurationRevisionsPath>, query: z.output<typeof zListConfigurationRevisionsQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/configuration/revisions", authenticated: true, successStatus: 200, success: zConfigurationRevisionList, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zListConfigurationRevisionsPath, value: path }, queryParameters: { schema: zListConfigurationRevisionsQuery, value: query } }, options);
}

export function listEnrollmentKeys(client: ControlClient, path: z.output<typeof zListEnrollmentKeysPath>, query: z.output<typeof zListEnrollmentKeysQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/enrollment-keys", authenticated: true, successStatus: 200, success: zEnrollmentKeysResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zListEnrollmentKeysPath, value: path }, queryParameters: { schema: zListEnrollmentKeysQuery, value: query } }, options);
}

export function listJoinLinks(client: ControlClient, path: z.output<typeof zListJoinLinksPath>, query: z.output<typeof zListJoinLinksQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/join-links", authenticated: true, successStatus: 200, success: zJoinLinkListResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zListJoinLinksPath, value: path }, queryParameters: { schema: zListJoinLinksQuery, value: query } }, options);
}

export function listNetworkActivity(client: ControlClient, path: z.output<typeof zListNetworkActivityPath>, query: z.output<typeof zListNetworkActivityQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/activity", authenticated: true, successStatus: 200, success: zActivityListResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zListNetworkActivityPath, value: path }, queryParameters: { schema: zListNetworkActivityQuery, value: query } }, options);
}

export function listNetworkMembers(client: ControlClient, path: z.output<typeof zListNetworkMembersPath>, query: z.output<typeof zListNetworkMembersQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/members", authenticated: true, successStatus: 200, success: zNetworkMembersResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zListNetworkMembersPath, value: path }, queryParameters: { schema: zListNetworkMembersQuery, value: query } }, options);
}

export function listNetworks(client: ControlClient, query: z.output<typeof zListNetworksQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks", authenticated: true, successStatus: 200, success: zNetworkListResponse, errors: [400, 401, 503], problem: zErrorResponse, queryParameters: { schema: zListNetworksQuery, value: query } }, options);
}

export function listNodes(client: ControlClient, path: z.output<typeof zListNodesPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/nodes", authenticated: true, successStatus: 200, success: zNodeInventoryResponse, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zListNodesPath, value: path } }, options);
}

export function listObservations(client: ControlClient, path: z.output<typeof zListObservationsPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/observations", authenticated: true, successStatus: 200, success: zObservationList, errors: [400, 401, 403, 404, 409, 422, 503], problem: zErrorResponse, pathParameters: { schema: zListObservationsPath, value: path } }, options);
}

export function listRuleSets(client: ControlClient, path: z.output<typeof zListRuleSetsPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/rule-sets", authenticated: true, successStatus: 200, success: zRuleSetList, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zListRuleSetsPath, value: path } }, options);
}

export function login(client: ControlClient, body: z.output<typeof zLoginRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/login", authenticated: false, successStatus: 200, success: zSessionIssuedResponse, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, body: { schema: zLoginRequest, value: body } }, options);
}

export function logout(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/auth/logout", authenticated: true, successStatus: 204, success: undefined, errors: [401, 503], problem: zErrorResponse }, options);
}

export function prepareUdpTraversal(client: ControlClient, body: z.output<typeof zPrepareUdpTraversalRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/connectivity/udp/attempts", authenticated: true, successStatus: 200, success: zPrepareUdpTraversalResponse, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, body: { schema: zPrepareUdpTraversalRequest, value: body } }, options);
}

export function queryEnrollmentRequest(client: ControlClient, path: z.output<typeof zQueryEnrollmentRequestPath>, body: z.output<typeof zEnrollmentDeviceProof>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/enrollment-requests/{enrollmentRequestId}/query", authenticated: false, successStatus: 200, success: zEnrollmentPollResponse, errors: [400, 401, 404, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zQueryEnrollmentRequestPath, value: path }, body: { schema: zEnrollmentDeviceProof, value: body } }, options);
}

export function queryObservationHistory(client: ControlClient, path: z.output<typeof zQueryObservationHistoryPath>, body: z.output<typeof zHistoryQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/history/query", authenticated: true, successStatus: 200, success: zHistoryPage, errors: [400, 401, 403, 404, 409, 413, 415, 429, 503], problem: zErrorResponse, pathParameters: { schema: zQueryObservationHistoryPath, value: path }, body: { schema: zHistoryQuery, value: body } }, options);
}

export function queryPeerState(client: ControlClient, body: z.output<typeof zStateQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/state/query", authenticated: true, successStatus: 200, success: zStateView, errors: [400, 401, 403, 404, 409, 413, 415, 429, 503], problem: zErrorResponse, body: { schema: zStateQuery, value: body } }, options);
}

export function queryUdpTraversal(client: ControlClient, body: z.output<typeof zQueryUdpTraversalRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/connectivity/udp/query", authenticated: true, successStatus: 200, success: zQueryUdpTraversalResponse, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, body: { schema: zQueryUdpTraversalRequest, value: body } }, options);
}

export function redeemEnrollmentRequest(client: ControlClient, path: z.output<typeof zRedeemEnrollmentRequestPath>, body: z.output<typeof zRedeemEnrollmentRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/enrollment-requests/{enrollmentRequestId}/redeem", authenticated: false, successStatus: 200, success: zEnrollmentResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zRedeemEnrollmentRequestPath, value: path }, body: { schema: zRedeemEnrollmentRequest, value: body } }, options);
}

export function redeemJoinLink(client: ControlClient, body: z.output<typeof zJoinLinkCredentialRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/join-links/redeem", authenticated: true, successStatus: 200, success: zJoinLinkRedeemed, errors: [400, 401, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zJoinLinkCredentialRequest, value: body } }, options);
}

export function redeemPasswordReset(client: ControlClient, body: z.output<typeof zPasswordResetRedeemRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/password-reset/redeem", authenticated: false, successStatus: 204, success: undefined, errors: [400, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zPasswordResetRedeemRequest, value: body } }, options);
}

export function refreshRuleSet(client: ControlClient, path: z.output<typeof zRefreshRuleSetPath>, body: z.output<typeof zRefreshRuleSetRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/rule-sets/{ruleSetKey}/refresh", authenticated: true, successStatus: 200, success: zRuleSetView, errors: [400, 401, 403, 404, 409, 413, 415, 429, 503], problem: zErrorResponse, pathParameters: { schema: zRefreshRuleSetPath, value: path }, body: { schema: zRefreshRuleSetRequest, value: body } }, options);
}

export function register(client: ControlClient, body: z.output<typeof zRegisterRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/register", authenticated: false, successStatus: 201, success: zSessionIssuedResponse, errors: [400, 403, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zRegisterRequest, value: body } }, options);
}

export function releaseNodeInstance(client: ControlClient, body: z.output<typeof zInstanceRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/instances/release", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zInstanceRequest, value: body } }, options);
}

export function removeMember(client: ControlClient, path: z.output<typeof zRemoveMemberPath>, body: z.output<typeof zRemoveMemberRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/members/{membershipId}/remove", authenticated: true, successStatus: 200, success: zMemberRemovedResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zRemoveMemberPath, value: path }, body: { schema: zRemoveMemberRequest, value: body } }, options);
}

export function renameNetwork(client: ControlClient, path: z.output<typeof zRenameNetworkPath>, body: z.output<typeof zRenameNetworkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/rename", authenticated: true, successStatus: 200, success: zNetworkResponse, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zRenameNetworkPath, value: path }, body: { schema: zRenameNetworkRequest, value: body } }, options);
}

export function renewNodeInstance(client: ControlClient, body: z.output<typeof zInstanceRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/instances/renew", authenticated: true, successStatus: 200, success: zInstanceLease, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zInstanceRequest, value: body } }, options);
}

export function renewNodeObservation(client: ControlClient, path: z.output<typeof zRenewNodeObservationPath>, body: z.output<typeof zRenewNodeObservation>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/observations/{subscriptionId}/renew", authenticated: true, successStatus: 200, success: zNodeObservationResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 429, 503], problem: zErrorResponse, pathParameters: { schema: zRenewNodeObservationPath, value: path }, body: { schema: zRenewNodeObservation, value: body } }, options);
}

export function renewObservation(client: ControlClient, path: z.output<typeof zRenewObservationPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/observations/{subscriptionId}/renew", authenticated: true, successStatus: 200, success: zObservationList, errors: [400, 401, 403, 404, 409, 422, 429, 503], problem: zErrorResponse, pathParameters: { schema: zRenewObservationPath, value: path } }, options);
}

export function reportDnsAssociation(client: ControlClient, body: z.output<typeof zDnsAssociationReport>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/dns-associations", authenticated: true, successStatus: 200, success: zDnsAssociationAccepted, errors: [400, 401, 403, 409, 413, 415, 429, 503], problem: zErrorResponse, body: { schema: zDnsAssociationReport, value: body } }, options);
}

export function reportNodeHistory(client: ControlClient, body: z.output<typeof zHistoryReport>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/history", authenticated: true, successStatus: 200, success: zHistoryAccepted, errors: [400, 401, 403, 404, 409, 413, 415, 429, 503], problem: zErrorResponse, body: { schema: zHistoryReport, value: body } }, options);
}

export function reportNodeState(client: ControlClient, body: z.output<typeof zStateReport>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/state", authenticated: true, successStatus: 200, success: zStateAccepted, errors: [400, 401, 403, 404, 409, 413, 415, 422, 429, 503], problem: zErrorResponse, body: { schema: zStateReport, value: body } }, options);
}

export function reportTaskResult(client: ControlClient, path: z.output<typeof zReportTaskResultPath>, body: z.output<typeof zObservationResult>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/tasks/{taskId}/results", authenticated: true, successStatus: 200, success: zObservationTask, errors: [400, 401, 403, 404, 409, 413, 415, 422, 429, 503], problem: zErrorResponse, pathParameters: { schema: zReportTaskResultPath, value: path }, body: { schema: zObservationResult, value: body } }, options);
}

export function revokeAccountSession(client: ControlClient, path: z.output<typeof zRevokeAccountSessionPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/sessions/{sessionId}/revoke", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zRevokeAccountSessionPath, value: path } }, options);
}

export function revokeAllAccountSessions(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/sessions/revoke-all", authenticated: true, successStatus: 204, success: undefined, errors: [401, 403, 503], problem: zErrorResponse }, options);
}

export function revokeEnrollmentKey(client: ControlClient, path: z.output<typeof zRevokeEnrollmentKeyPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "DELETE", path: "/api/v1/networks/{networkId}/enrollment-keys/{keyId}", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 404, 409, 503], problem: zErrorResponse, pathParameters: { schema: zRevokeEnrollmentKeyPath, value: path } }, options);
}

export function revokeJoinLink(client: ControlClient, path: z.output<typeof zRevokeJoinLinkPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/join-links/{linkId}/revoke", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 404, 409, 503], problem: zErrorResponse, pathParameters: { schema: zRevokeJoinLinkPath, value: path } }, options);
}

export function revokeNodeKey(client: ControlClient, path: z.output<typeof zRevokeNodeKeyPath>, body: z.output<typeof zRevokeNodeKeyRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/nodes/{nodeId}/keys/revoke", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zRevokeNodeKeyPath, value: path }, body: { schema: zRevokeNodeKeyRequest, value: body } }, options);
}

export function revokeOtherAccountSessions(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/sessions/revoke-others", authenticated: true, successStatus: 204, success: undefined, errors: [401, 403, 503], problem: zErrorResponse }, options);
}

export function rotateNodeKeys(client: ControlClient, body: z.output<typeof zNodeRotationRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/agent-api/v1/auth/rotate", authenticated: false, successStatus: 200, success: zNodeKeyMetadata, errors: [400, 401, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zNodeRotationRequest, value: body } }, options);
}

export function selectNodeRoutingProfile(client: ControlClient, body: z.output<typeof zRoutingSelectionRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/routing-selection", authenticated: true, successStatus: 200, success: zRoutingSelectionResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zRoutingSelectionRequest, value: body } }, options);
}

export function submitNodeConfigurationReceipt(client: ControlClient, body: z.output<typeof zReceiptRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/configuration/receipts", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zReceiptRequest, value: body } }, options);
}

export function syncNode(client: ControlClient, body: z.output<typeof zSyncRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/agent-api/v1/sync", authenticated: true, successStatus: 200, success: zSyncResponse, errors: [400, 401, 403, 413, 415, 503], problem: zErrorResponse, body: { schema: zSyncRequest, value: body } }, options);
}

export function validateConfiguration(client: ControlClient, path: z.output<typeof zValidateConfigurationPath>, body: z.output<typeof zConfigurationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/configuration/validate", authenticated: true, successStatus: 200, success: zConfigurationPreviewResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zValidateConfigurationPath, value: path }, body: { schema: zConfigurationRequest, value: body } }, options);
}

export function watchNetwork(client: ControlClient, path: z.output<typeof zWatchNetworkPath>, options: AuthenticatedStreamOptions) {
  return client.stream({ method: "GET", path: "/api/v1/networks/{networkId}/events", authenticated: true, successStatus: 200, success: zUserEvent, errors: [400, 401, 403, 404, 409, 429, 503], problem: zErrorResponse, pathParameters: { schema: zWatchNetworkPath, value: path } }, options);
}
