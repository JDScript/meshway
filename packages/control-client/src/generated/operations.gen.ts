// Generated from Controller OpenAPI. Do not edit.

import type * as z from "zod";
import { ControlClient, type AuthenticatedRequestOptions, type RequestOptions } from "../transport.ts";
import { zAuthOptionsResponse, zAuthSessionResponse, zCreateNetworkRequest, zErrorResponse, zGetNetworkPath, zListNetworkMembersPath, zListNetworkMembersQuery, zListNetworksQuery, zLoginRequest, zNetworkListResponse, zNetworkMembersResponse, zNetworkResponse, zRegisterRequest, zRenameNetworkPath, zRenameNetworkRequest, zSessionIssuedResponse } from "./zod.gen.ts";

export function createNetwork(client: ControlClient, body: z.output<typeof zCreateNetworkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks", authenticated: true, successStatus: 201, success: zNetworkResponse, errors: [400, 401, 404, 409, 413, 415, 503], problem: zErrorResponse, body: { schema: zCreateNetworkRequest, value: body } }, options);
}

export function getAuthOptions(client: ControlClient, options: RequestOptions = {}) {
  return client.request({ method: "GET", path: "/api/v1/auth/options", authenticated: false, successStatus: 200, success: zAuthOptionsResponse, errors: [503], problem: zErrorResponse }, options);
}

export function getAuthSession(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/auth/session", authenticated: true, successStatus: 200, success: zAuthSessionResponse, errors: [401, 503], problem: zErrorResponse }, options);
}

export function getNetwork(client: ControlClient, path: z.output<typeof zGetNetworkPath>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/networks/{networkId}", authenticated: true, successStatus: 200, success: zNetworkResponse, errors: [400, 401, 404, 503], problem: zErrorResponse, pathParameters: { schema: zGetNetworkPath, value: path } }, options);
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

export function register(client: ControlClient, body: z.output<typeof zRegisterRequest>, options: RequestOptions = {}) {
  return client.request({ method: "POST", path: "/api/v1/auth/register", authenticated: false, successStatus: 201, success: zSessionIssuedResponse, errors: [400, 403, 409, 413, 415, 422, 503], problem: zErrorResponse, body: { schema: zRegisterRequest, value: body } }, options);
}

export function renameNetwork(client: ControlClient, path: z.output<typeof zRenameNetworkPath>, body: z.output<typeof zRenameNetworkRequest>, options: AuthenticatedRequestOptions) {
  return client.request({ method: "POST", path: "/api/v1/networks/{networkId}/rename", authenticated: true, successStatus: 200, success: zNetworkResponse, errors: [400, 401, 403, 404, 409, 413, 415, 503], problem: zErrorResponse, pathParameters: { schema: zRenameNetworkPath, value: path }, body: { schema: zRenameNetworkRequest, value: body } }, options);
}
