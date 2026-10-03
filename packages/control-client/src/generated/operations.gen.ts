// Generated from Controller OpenAPI. Do not edit.

import type * as z from "zod";
import { ControlClient, type AuthenticatedRequestOptions, type RequestOptions } from "../transport.ts";
import { zAuthOptionsResponse, zAuthSessionResponse, zErrorResponse, zLoginRequest, zRegisterRequest, zSessionIssuedResponse } from "./zod.gen.ts";

export function getAuthOptions(client: ControlClient, options: RequestOptions = {}) {
  return client.request({ method: "GET", path: "/api/v1/auth/options", authenticated: false, successStatus: 200, success: zAuthOptionsResponse, errors: [503], problem: zErrorResponse }, options);
}

export function getAuthSession(client: ControlClient, options: AuthenticatedRequestOptions) {
  return client.request({ method: "GET", path: "/api/v1/auth/session", authenticated: true, successStatus: 200, success: zAuthSessionResponse, errors: [401, 503], problem: zErrorResponse }, options);
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
