// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import * as z from "zod";

export const zNodeSummary = z.strictObject({
  "agent_version": z.string(),
  "id": z.string(),
  "last_seen": z.iso.datetime({ offset: true }),
  "name": z.string(),
  "registered_at": z.iso.datetime({ offset: true }),
  "revoked_at": z.iso.datetime({ offset: true }).nullable()
});

export const zAgentNodeResponse = z.strictObject({
  "node": zNodeSummary
});

export const zCreateEnrollmentTokenRequest = z.strictObject({
  "expires_in_seconds": z.int().gte(60).lte(86400)
});

export const zCreateEnrollmentTokenResponse = z.strictObject({
  "expires_at": z.iso.datetime({ offset: true }),
  "token": z.string(),
  "token_id": z.string()
});

export const zEnrollAgentRequest = z.strictObject({
  "agent_version": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "node_credential": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u"))
});

export const zErrorResponse = z.strictObject({
  "message": z.string()
});

export const zHeartbeatRequest = z.strictObject({
  "agent_version": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u"))
});

export const zNodeList = z.strictObject({
  "nodes": z.array(zNodeSummary)
});

export const zOverview = z.strictObject({
  "name": z.string(),
  "node_count": z.int().gte(0).lte(4294967295),
  "version": z.string()
});

export const zRevokeNodeRequest = z.strictObject({
  "node_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points")
});

export const zRevokeNodeResponse = z.strictObject({
  "node": zNodeSummary
});
