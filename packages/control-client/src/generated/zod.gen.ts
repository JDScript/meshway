// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import * as z from "zod";

export const zAcknowledgeConfigurationRequest = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "revision": z.int().gte(1).lte(4294967295)
});

export const zAcknowledgeConfigurationResponse = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "revision": z.int().gte(1).lte(4294967295)
});

export const zNodeSummary = z.strictObject({
  "acknowledged_revision": z.int().gte(0).lte(4294967295),
  "agent_version": z.string(),
  "configuration_revision": z.int().gte(1).lte(4294967295),
  "country": z.string().nullable(),
  "id": z.string(),
  "last_seen": z.iso.datetime({ offset: true }),
  "mesh_ipv4": z.string(),
  "mesh_ipv6": z.string(),
  "name": z.string(),
  "registered_at": z.iso.datetime({ offset: true }),
  "revoked_at": z.iso.datetime({ offset: true }).nullable()
});

export const zAgentNodeResponse = z.strictObject({
  "node": zNodeSummary
});

export const zPolicyDiagnostic = z.strictObject({
  "field": z.string(),
  "message": z.string(),
  "rule_id": z.string().nullable()
});

export const zMeshNetwork = z.strictObject({
  "ipv4_prefix": z.string(),
  "ipv6_prefix": z.string()
});

export const zMeshNode = z.strictObject({
  "country": z.string().nullable(),
  "id": z.string(),
  "mesh_ipv4": z.string(),
  "mesh_ipv6": z.string(),
  "name": z.string()
});

export const zDefaultRouteAction = z.enum(["direct","reject"]);

export const zRouteActionKind = z.enum(["route","reject"]);

export const zRouteAction = z.strictObject({
  "exit_node_id": z.string().nullable(),
  "kind": zRouteActionKind,
  "via": z.array(z.string())
});

export const zDestinationSelectorKind = z.enum(["node","domain_suffix","cidr","country"]);

export const zDestinationSelector = z.strictObject({
  "kind": zDestinationSelectorKind,
  "value": z.string()
});

export const zDestinationScope = z.enum(["any","mesh","internet"]);

export const zSourceSelector = z.strictObject({
  "countries": z.array(z.string()),
  "node_ids": z.array(z.string())
});

export const zRouteRule = z.strictObject({
  "action": zRouteAction,
  "destination_any": z.array(zDestinationSelector),
  "destination_scope": zDestinationScope,
  "id": z.string(),
  "source": zSourceSelector
});

export const zRoutingPolicy = z.strictObject({
  "default_action": zDefaultRouteAction,
  "rules": z.array(zRouteRule)
});

export const zNodeConfiguration = z.strictObject({
  "diagnostics": z.array(zPolicyDiagnostic),
  "network": zMeshNetwork,
  "node": zMeshNode,
  "peers": z.array(zMeshNode),
  "policy_valid": z.boolean(),
  "revision": z.int().gte(1).lte(4294967295),
  "routing_policy": zRoutingPolicy,
  "schema_version": z.int().gte(1).lte(4294967295)
});

export const zConfigurationSnapshot = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "payload": zNodeConfiguration
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

export const zFlowQuery = z.strictObject({
  "destination_country": z.string().nullable(),
  "destination_ip": z.string().nullable(),
  "destination_node_id": z.string().nullable(),
  "domain": z.string().nullable(),
  "source_node_id": z.string()
});

export const zHeartbeatRequest = z.strictObject({
  "agent_version": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u"))
});

export const zNodeList = z.strictObject({
  "nodes": z.array(zNodeSummary)
});

export const zOverview = z.strictObject({
  "configuration_revision": z.int().gte(1).lte(4294967295),
  "name": z.string(),
  "node_count": z.int().gte(0).lte(4294967295),
  "version": z.string()
});

export const zPolicySnapshot = z.strictObject({
  "diagnostics": z.array(zPolicyDiagnostic),
  "network": zMeshNetwork,
  "policy": zRoutingPolicy,
  "revision": z.int().gte(1).lte(4294967295),
  "valid": z.boolean()
});

export const zPreviewPolicyRequest = z.strictObject({
  "flow": zFlowQuery,
  "policy": zRoutingPolicy
});

export const zRouteDecisionAction = z.enum(["route","reject","invalid"]);

export const zRouteDecision = z.strictObject({
  "action": zRouteDecisionAction,
  "exit_node_id": z.string().nullable(),
  "path": z.array(z.string()),
  "reason": z.string(),
  "rule_id": z.string().nullable()
});

export const zPreviewPolicyResponse = z.strictObject({
  "decision": zRouteDecision,
  "diagnostics": z.array(zPolicyDiagnostic),
  "revision": z.int().gte(1).lte(4294967295),
  "valid": z.boolean()
});

export const zRevokeNodeRequest = z.strictObject({
  "node_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points")
});

export const zRevokeNodeResponse = z.strictObject({
  "node": zNodeSummary
});

export const zSavePolicyRequest = z.strictObject({
  "expected_revision": z.int().gte(1).lte(4294967295),
  "policy": zRoutingPolicy
});

export const zSetNodeCountryRequest = z.strictObject({
  "country": z.string().nullable(),
  "expected_revision": z.int().gte(1).lte(4294967295),
  "node_id": z.string()
});
