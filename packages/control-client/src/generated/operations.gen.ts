// Generated from Controller OpenAPI. Do not edit.

import type * as z from "zod";
import { ControlClient, type AuthenticatedRequestOptions, type RequestOptions } from "../transport.ts";
import { zAccountResponse, zAccountSessionsResponse, zActivityListResponse, zActivityResponse, zApplyConfigurationPath, zAuthOptionsResponse, zAuthSessionResponse, zChangeEmailRequest, zChangeMemberRolePath, zChangeMemberRoleRequest, zChangePasswordRequest, zCompareConfigurationPath, zCompareConfigurationRequest, zConfigurationAppliedResponse, zConfigurationComparison, zConfigurationPreviewResponse, zConfigurationRequest, zConfigurationResponse, zConfigurationRevisionList, zConfigurationSchemaResponse, zConfigurationSourceResponse, zCreateJoinLinkPath, zCreateJoinLinkRequest, zCreateNetworkRequest, zErrorResponse, zGetConfigurationPath, zGetConfigurationRevisionPath, zGetConfigurationSourcePath, zGetConfigurationSourceQuery, zGetLeaveImpactPath, zGetMemberRemovalImpactPath, zGetNetworkActivityPath, zGetNetworkPath, zJoinLinkCreated, zJoinLinkCredentialRequest, zJoinLinkInspection, zJoinLinkListResponse, zJoinLinkRedeemed, zLeaveNetworkPath, zListAccountActivityQuery, zListAccountSessionsQuery, zListConfigurationRevisionsPath, zListConfigurationRevisionsQuery, zListJoinLinksPath, zListJoinLinksQuery, zListNetworkActivityPath, zListNetworkActivityQuery, zListNetworkMembersPath, zListNetworkMembersQuery, zListNetworksQuery, zLoginRequest, zMemberMutationResponse, zMemberRemovedResponse, zNetworkListResponse, zNetworkMembersResponse, zNetworkResponse, zPasswordResetInspectRequest, zPasswordResetInspection, zPasswordResetRedeemRequest, zRegisterRequest, zRemovalImpactResponse, zRemoveMemberPath, zRemoveMemberRequest, zRenameNetworkPath, zRenameNetworkRequest, zRevokeAccountSessionPath, zRevokeJoinLinkPath, zSessionIssuedResponse, zTransportCatalogResponse, zValidateConfigurationPath } from "./zod.gen.ts";

export function applyConfiguration(client: ControlClient, path: z.output<typeof zApplyConfigurationPath>, body: z.output<typeof zConfigurationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/configuration", authenticated: true, successStatus: 200, success: zConfigurationAppliedResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zApplyConfigurationPath, value: path }, body: { schema: zConfigurationRequest, value: body } }, options);
}

export function changeAccountEmail(client: ControlClient, body: z.output<typeof zChangeEmailRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/email", authenticated: true, successStatus: 200, success: zAccountResponse, errors: [400, 401, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zChangeEmailRequest, value: body } }, options);
}

export function changeAccountPassword(client: ControlClient, body: z.output<typeof zChangePasswordRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/password", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zChangePasswordRequest, value: body } }, options);
}

export function changeMemberRole(client: ControlClient, path: z.output<typeof zChangeMemberRolePath>, body: z.output<typeof zChangeMemberRoleRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/members/{membershipId}/role", authenticated: true, successStatus: 200, success: zMemberMutationResponse, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zChangeMemberRolePath, value: path }, body: { schema: zChangeMemberRoleRequest, value: body } }, options);
}

export function compareConfiguration(client: ControlClient, path: z.output<typeof zCompareConfigurationPath>, body: z.output<typeof zCompareConfigurationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/configuration/compare", authenticated: true, successStatus: 200, success: zConfigurationComparison, errors: [400, 401, 403, 404, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zCompareConfigurationPath, value: path }, body: { schema: zCompareConfigurationRequest, value: body } }, options);
}

export function createJoinLink(client: ControlClient, path: z.output<typeof zCreateJoinLinkPath>, body: z.output<typeof zCreateJoinLinkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/join-links", authenticated: true, successStatus: 201, success: zJoinLinkCreated, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zCreateJoinLinkPath, value: path }, body: { schema: zCreateJoinLinkRequest, value: body } }, options);
}

export function createNetwork(client: ControlClient, body: z.output<typeof zCreateNetworkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks", authenticated: true, successStatus: 201, success: zNetworkResponse, errors: [400, 401, 404, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zCreateNetworkRequest, value: body } }, options);
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

export function getConfigurationRevision(client: ControlClient, path: z.output<typeof zGetConfigurationRevisionPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/configuration/revisions/{revision}", authenticated: true, successStatus: 200, success: zConfigurationResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetConfigurationRevisionPath, value: path } }, options);
}

export function getConfigurationSchema(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/configuration/schema", authenticated: true, successStatus: 200, success: zConfigurationSchemaResponse, errors: [401, 503], problem: zErrorResponse }, options);
}

export function getConfigurationSource(client: ControlClient, path: z.output<typeof zGetConfigurationSourcePath>, query: z.output<typeof zGetConfigurationSourceQuery>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/configuration/source", authenticated: true, successStatus: 200, success: zConfigurationSourceResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetConfigurationSourcePath, value: path }, queryParameters: { schema: zGetConfigurationSourceQuery, value: query } }, options);
}

export function getLeaveImpact(client: ControlClient, path: z.output<typeof zGetLeaveImpactPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/leave-impact", authenticated: true, successStatus: 200, success: zRemovalImpactResponse, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetLeaveImpactPath, value: path } }, options);
}

export function getMemberRemovalImpact(client: ControlClient, path: z.output<typeof zGetMemberRemovalImpactPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/members/{membershipId}/removal-impact", authenticated: true, successStatus: 200, success: zRemovalImpactResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetMemberRemovalImpactPath, value: path } }, options);
}

export function getNetwork(client: ControlClient, path: z.output<typeof zGetNetworkPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}", authenticated: true, successStatus: 200, success: zNetworkResponse, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetNetworkPath, value: path } }, options);
}

export function getNetworkActivity(client: ControlClient, path: z.output<typeof zGetNetworkActivityPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}/activity/{activityId}", authenticated: true, successStatus: 200, success: zActivityResponse, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetNetworkActivityPath, value: path } }, options);
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
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/leave", authenticated: true, successStatus: 200, success: zMemberRemovedResponse, errors: [400, 401, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zLeaveNetworkPath, value: path }, body: { schema: zRemoveMemberRequest, value: body } }, options);
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

export function login(client: ControlClient, body: z.output<typeof zLoginRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/login", authenticated: false, successStatus: 200, success: zSessionIssuedResponse, errors: [400, 401, 413, 415, 503], problem: zErrorResponse, body: { schema: zLoginRequest, value: body } }, options);
}

export function logout(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/auth/logout", authenticated: true, successStatus: 204, success: undefined, errors: [401, 503], problem: zErrorResponse }, options);
}

export function redeemJoinLink(client: ControlClient, body: z.output<typeof zJoinLinkCredentialRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/join-links/redeem", authenticated: true, successStatus: 200, success: zJoinLinkRedeemed, errors: [400, 401, 404, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zJoinLinkCredentialRequest, value: body } }, options);
}

export function redeemPasswordReset(client: ControlClient, body: z.output<typeof zPasswordResetRedeemRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/password-reset/redeem", authenticated: false, successStatus: 204, success: undefined, errors: [400, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zPasswordResetRedeemRequest, value: body } }, options);
}

export function register(client: ControlClient, body: z.output<typeof zRegisterRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/register", authenticated: false, successStatus: 201, success: zSessionIssuedResponse, errors: [400, 403, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zRegisterRequest, value: body } }, options);
}

export function removeMember(client: ControlClient, path: z.output<typeof zRemoveMemberPath>, body: z.output<typeof zRemoveMemberRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/members/{membershipId}/remove", authenticated: true, successStatus: 200, success: zMemberRemovedResponse, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zRemoveMemberPath, value: path }, body: { schema: zRemoveMemberRequest, value: body } }, options);
}

export function renameNetwork(client: ControlClient, path: z.output<typeof zRenameNetworkPath>, body: z.output<typeof zRenameNetworkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/rename", authenticated: true, successStatus: 200, success: zNetworkResponse, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zRenameNetworkPath, value: path }, body: { schema: zRenameNetworkRequest, value: body } }, options);
}

export function revokeAccountSession(client: ControlClient, path: z.output<typeof zRevokeAccountSessionPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/sessions/{sessionId}/revoke", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zRevokeAccountSessionPath, value: path } }, options);
}

export function revokeAllAccountSessions(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/sessions/revoke-all", authenticated: true, successStatus: 204, success: undefined, errors: [401, 503], problem: zErrorResponse }, options);
}

export function revokeJoinLink(client: ControlClient, path: z.output<typeof zRevokeJoinLinkPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/join-links/{linkId}/revoke", authenticated: true, successStatus: 204, success: undefined, errors: [400, 401, 403, 404, 503], problem: zErrorResponse, pathParameters: { schema: zRevokeJoinLinkPath, value: path } }, options);
}

export function revokeOtherAccountSessions(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/account/sessions/revoke-others", authenticated: true, successStatus: 204, success: undefined, errors: [401, 503], problem: zErrorResponse }, options);
}

export function validateConfiguration(client: ControlClient, path: z.output<typeof zValidateConfigurationPath>, body: z.output<typeof zConfigurationRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/configuration/validate", authenticated: true, successStatus: 200, success: zConfigurationPreviewResponse, errors: [400, 401, 403, 404, 409, 413, 415, 422, 503], problem: zErrorResponse, pathParameters: { schema: zValidateConfigurationPath, value: path }, body: { schema: zConfigurationRequest, value: body } }, options);
}
