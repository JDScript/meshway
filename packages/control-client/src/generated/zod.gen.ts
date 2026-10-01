// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import * as z from "zod";

export const zAccessAction = z.enum(["allow","reject"]);

export const zAcknowledgeAgentConnectionsRequest = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "revision": z.int().gte(1).lte(4294967295)
});

export const zConnectionReceiptStatus = z.enum(["saved"]);

export const zAcknowledgeAgentConnectionsResponse = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "revision": z.int().gte(1).lte(4294967295),
  "status": zConnectionReceiptStatus
});

export const zAcknowledgeConfigurationRequest = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "revision": z.int().gte(1).lte(4294967295)
});

export const zAcknowledgeConfigurationResponse = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "revision": z.int().gte(1).lte(4294967295)
});

export const zConnectionSemantics = z.enum(["datagram","stream"]);

export const zConnectionTransport = z.enum(["native_udp","native_tcp","vless"]);

export const zConnectionCapability = z.strictObject({
  "can_accept": z.boolean(),
  "can_dial": z.boolean(),
  "semantics": zConnectionSemantics,
  "transport": zConnectionTransport
});

export const zConnectionDiagnostic = z.strictObject({
  "field": z.string(),
  "item_id": z.string().nullable(),
  "message": z.string()
});

export const zIncomingConnectionApproval = z.strictObject({
  "candidate_id": z.string(),
  "listener_id": z.string(),
  "peer_node_id": z.string(),
  "transport": zConnectionTransport
});

export const zConnectionEndpoint = z.strictObject({
  "host": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 253, "maxLength 253 Unicode code points"),
  "port": z.int().gte(1).lte(65535)
});

export const zConnectionListener = z.strictObject({
  "advertised_endpoints": z.array(zConnectionEndpoint).min(1).max(16),
  "bind": z.string(),
  "credential_ref": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")).nullable(),
  "id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "node_id": z.string(),
  "transport": zConnectionTransport
});

export const zOutgoingConnectionCandidate = z.strictObject({
  "credential_ref": z.string().nullable(),
  "endpoint": zConnectionEndpoint,
  "id": z.string(),
  "listener_id": z.string(),
  "peer_node_id": z.string(),
  "priority": z.int().gte(0).lte(65535),
  "transport": zConnectionTransport
});

export const zConnectionPeer = z.strictObject({
  "node_id": z.string(),
  "public_key": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u"))
});

export const zNodeConnections = z.strictObject({
  "capabilities": z.array(zConnectionCapability),
  "diagnostics": z.array(zConnectionDiagnostic),
  "incoming": z.array(zIncomingConnectionApproval),
  "listeners": z.array(zConnectionListener),
  "node_id": z.string(),
  "outgoing": z.array(zOutgoingConnectionCandidate),
  "peers": z.array(zConnectionPeer),
  "public_key": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")).nullable(),
  "revision": z.int().gte(1).lte(4294967295),
  "schema_version": z.int().gte(1).lte(4294967295),
  "valid": z.boolean()
});

export const zAgentConnectionsSnapshot = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "payload": zNodeConnections
});

export const zMeshAccessGrant = z.strictObject({
  "id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "path": z.array(z.string()).min(2).max(8)
});

export const zMeshPathAuthorization = z.strictObject({
  "grants": z.array(zMeshAccessGrant).max(128)
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

export const zNodeDataplane = z.strictObject({
  "access_policy": zMeshPathAuthorization,
  "access_valid": z.boolean(),
  "configuration": zNodeConfiguration,
  "connections": zNodeConnections,
  "identities": z.array(zConnectionPeer).max(64),
  "node_id": z.string(),
  "revision": z.int().gte(1).lte(4294967295),
  "schema_version": z.int().gte(1).lte(1)
});

export const zAgentDataplaneSnapshot = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "payload": zNodeDataplane
});

export const zMeshDnsConfiguration = z.strictObject({
  "enabled": z.boolean(),
  "ttl_seconds": z.int().gte(1).lte(300),
  "zone": z.string().refine(value => [...value].length >= 3, "minLength 3 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points")
});

export const zNodeDnsRecord = z.strictObject({
  "aliases": z.array(z.string()).max(1),
  "canonical_name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 253, "maxLength 253 Unicode code points").regex(new RegExp("^id\\.[0-9a-f]{32}\\.[0-9a-f]{32}\\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$", "u")),
  "mesh_ipv4": z.string(),
  "mesh_ipv6": z.string(),
  "node_id": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u"))
});

export const zNodeDnsConfiguration = z.strictObject({
  "diagnostics": z.array(zPolicyDiagnostic).max(64),
  "dns": zMeshDnsConfiguration,
  "network_id": z.string().refine(value => [...value].length >= 32, "minLength 32 Unicode code points").refine(value => [...value].length <= 32, "maxLength 32 Unicode code points").regex(new RegExp("^[0-9a-f]{32}$", "u")),
  "node_id": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "records": z.array(zNodeDnsRecord).max(1024),
  "revision": z.int().gte(1).lte(4294967295),
  "schema_version": z.int().gte(1).lte(1)
});

export const zAgentDnsSnapshot = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "payload": zNodeDnsConfiguration
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

export const zAuthorizeDeviceRequest = z.strictObject({
  "agent_version": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "code_challenge": z.string().refine(value => [...value].length >= 43, "minLength 43 Unicode code points").refine(value => [...value].length <= 43, "maxLength 43 Unicode code points").regex(new RegExp("^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$", "u")),
  "name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "redirect_uri": z.string().refine(value => [...value].length >= 25, "minLength 25 Unicode code points").refine(value => [...value].length <= 25, "maxLength 25 Unicode code points").regex(new RegExp("^meshway://enroll/callback$", "u"))
});

export const zAuthorizeDeviceResponse = z.strictObject({
  "code": z.string(),
  "expires_at": z.iso.datetime({ offset: true })
});

export const zBeginConnectionRuntimeRequest = z.strictObject({
  "expected_epoch": z.int().gte(0).lte(4294967295),
  "instance_id": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u"))
});

export const zBeginConnectionRuntimeResponse = z.strictObject({
  "epoch": z.int().gte(1).lte(4294967295)
});

export const zConfigurationSnapshot = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "payload": zNodeConfiguration
});

export const zConnectionCandidate = z.strictObject({
  "credential_ref": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")).nullable(),
  "dialer_node_id": z.string(),
  "endpoint": zConnectionEndpoint,
  "id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "listener_id": z.string(),
  "priority": z.int().gte(0).lte(65535)
});

export const zConnectionRuntimeReason = z.enum(["none","disabled","invalid_configuration","profile_unavailable","bind_failed","resolve_failed","transport_failed","authentication_failed","probe_failed","lease_superseded","lease_expired","shutting_down","internal_error"]);

export const zConnectionListenerState = z.enum(["bound","error"]);

export const zConnectionListenerObservation = z.strictObject({
  "listener_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "reason": zConnectionRuntimeReason,
  "state": zConnectionListenerState
});

export const zConnectionNeighborState = z.enum(["authenticated","waiting","unreachable"]);

export const zConnectionNeighborObservation = z.strictObject({
  "candidate_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")).nullable(),
  "initiator_node_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "listener_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")).nullable(),
  "peer_node_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "probe_fresh": z.boolean(),
  "probe_rtt_ms": z.int().gte(0).lte(4294967295).nullable(),
  "reason": zConnectionRuntimeReason,
  "state": zConnectionNeighborState,
  "transport": zConnectionTransport
});

export const zConnectionNode = z.strictObject({
  "acknowledged_at": z.iso.datetime({ offset: true }).nullable(),
  "acknowledged_digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")).nullable(),
  "acknowledged_revision": z.int().gte(0).lte(4294967295),
  "capabilities": z.array(zConnectionCapability),
  "node": zNodeSummary,
  "public_key": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")).nullable()
});

export const zConnectionPlan = z.strictObject({
  "candidates": z.array(zConnectionCandidate).max(256),
  "listeners": z.array(zConnectionListener).max(128)
});

export const zConnectionRuntimeState = z.enum(["awaiting_report","disabled","applying","applied","error","withdrawn"]);

export const zConnectionRuntimeObservation = z.strictObject({
  "current": z.boolean(),
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")).nullable(),
  "epoch": z.int().gte(1).lte(4294967295),
  "listeners": z.array(zConnectionListenerObservation).max(128),
  "node_id": z.string(),
  "peers": z.array(zConnectionNeighborObservation).max(256),
  "reason": zConnectionRuntimeReason,
  "reported_at": z.iso.datetime({ offset: true }).nullable(),
  "revision": z.int().gte(1).lte(4294967295).nullable(),
  "sequence": z.int().gte(0).lte(4294967295),
  "stale": z.boolean(),
  "stale_after_seconds": z.int().gte(1).lte(4294967295),
  "state": zConnectionRuntimeState
});

export const zConnectionRuntimesSnapshot = z.strictObject({
  "nodes": z.array(zConnectionRuntimeObservation)
});

export const zConnectionsSnapshot = z.strictObject({
  "configuration": zConnectionPlan,
  "diagnostics": z.array(zConnectionDiagnostic),
  "nodes": z.array(zConnectionNode),
  "revision": z.int().gte(1).lte(4294967295),
  "valid": z.boolean()
});

export const zCreateEnrollmentTokenRequest = z.strictObject({
  "expires_in_seconds": z.int().gte(60).lte(86400)
});

export const zCreateEnrollmentTokenResponse = z.strictObject({
  "expires_at": z.iso.datetime({ offset: true }),
  "token": z.string(),
  "token_id": z.string()
});

export const zCreateNetworkRequest = z.strictObject({
  "name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u"))
});

export const zStunDiscoveryOutcome = z.enum(["mapped","timeout","invalid_response","io_error","stun_error"]);

export const zStunEndpointObservation = z.strictObject({
  "expires_at": z.iso.datetime({ offset: true }),
  "mapped_endpoint": z.string().refine(value => [...value].length >= 3, "minLength 3 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").nullable(),
  "observed_at": z.iso.datetime({ offset: true }),
  "observer": z.string().refine(value => [...value].length >= 3, "minLength 3 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points"),
  "outcome": zStunDiscoveryOutcome,
  "rtt_ms": z.int().gte(0).lte(4294967295).nullable()
});

export const zUdpDiscoveryResourceKind = z.enum(["listener","outgoing_candidate"]);

export const zEndpointDiscoverySource = z.enum(["managed_udp_socket_stun"]);

export const zUdpSocketDiscovery = z.strictObject({
  "bound_address": z.string().refine(value => [...value].length >= 3, "minLength 3 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points"),
  "generation": z.int().gte(1).lte(4294967295),
  "observations": z.array(zStunEndpointObservation).max(4),
  "resource_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "resource_kind": zUdpDiscoveryResourceKind,
  "socket_id": z.int().gte(1).lte(4294967295),
  "source": zEndpointDiscoverySource
});

export const zEndpointDiscoveryState = z.enum(["awaiting_report","disabled","probing","observed","withdrawn","expired","error"]);

export const zNodeEndpointDiscovery = z.strictObject({
  "current": z.boolean(),
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")).nullable(),
  "epoch": z.int().gte(0).lte(4294967295),
  "node_id": z.string(),
  "reported_at": z.iso.datetime({ offset: true }).nullable(),
  "revision": z.int().gte(1).lte(4294967295).nullable(),
  "sequence": z.int().gte(0).lte(4294967295),
  "sockets": z.array(zUdpSocketDiscovery).max(128),
  "stale": z.boolean(),
  "state": zEndpointDiscoveryState
});

export const zEndpointDiscoveriesSnapshot = z.strictObject({
  "network_id": z.string(),
  "nodes": z.array(zNodeEndpointDiscovery)
});

export const zEnrollAgentRequest = z.strictObject({
  "agent_version": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "node_credential": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u"))
});

export const zErrorResponse = z.strictObject({
  "message": z.string()
});

export const zExchangeDeviceRequest = z.strictObject({
  "agent_version": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "code_verifier": z.string().refine(value => [...value].length >= 43, "minLength 43 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("^[A-Za-z0-9._~-]{43,128}$", "u")),
  "name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "node_credential": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "redirect_uri": z.string().refine(value => [...value].length >= 25, "minLength 25 Unicode code points").refine(value => [...value].length <= 25, "maxLength 25 Unicode code points").regex(new RegExp("^meshway://enroll/callback$", "u"))
});

export const zFlowAuthorizationAction = z.enum(["allow","reject","invalid"]);

export const zFlowAuthorizationDecision = z.strictObject({
  "action": zFlowAuthorizationAction,
  "matched_direction": z.string().regex(new RegExp("^(forward|reverse)$", "u")).nullable(),
  "reason": z.string(),
  "rule_id": z.string().nullable()
});

export const zFlowQuery = z.strictObject({
  "destination_country": z.string().nullable(),
  "destination_ip": z.string().nullable(),
  "destination_node_id": z.string().nullable(),
  "domain": z.string().nullable(),
  "source_node_id": z.string()
});

export const zGetConnectionRuntimeEpochResponse = z.strictObject({
  "epoch": z.int().gte(0).lte(4294967295)
});

export const zHeartbeatRequest = z.strictObject({
  "agent_version": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u"))
});

export const zMeshAccessRule = z.strictObject({
  "action": zAccessAction,
  "bidirectional": z.boolean(),
  "destination": z.array(z.string()).min(1).max(32),
  "id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9._-]+$", "u")),
  "source": z.array(z.string()).min(1).max(32)
});

export const zMeshAccessPolicy = z.strictObject({
  "default_action": zAccessAction,
  "rules": z.array(zMeshAccessRule).max(128)
});

export const zMeshAccessSnapshot = z.strictObject({
  "diagnostics": z.array(zPolicyDiagnostic),
  "policy": zMeshAccessPolicy,
  "revision": z.int().gte(1).lte(4294967295),
  "source_jsonc": z.string().nullable(),
  "valid": z.boolean()
});

export const zNetworkSnapshot = z.strictObject({
  "dns": zMeshDnsConfiguration,
  "name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u")),
  "network": zMeshNetwork,
  "network_id": z.string().refine(value => [...value].length >= 32, "minLength 32 Unicode code points").refine(value => [...value].length <= 32, "maxLength 32 Unicode code points").regex(new RegExp("^[0-9a-f]{32}$", "u")),
  "revision": z.int().gte(1).lte(4294967295)
});

export const zNetworkList = z.strictObject({
  "networks": z.array(zNetworkSnapshot)
});

export const zNodeList = z.strictObject({
  "nodes": z.array(zNodeSummary)
});

export const zNodeTagAssignment = z.strictObject({
  "node_id": z.string(),
  "tags": z.array(z.string()).max(32)
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
  "source_jsonc": z.string().nullable(),
  "valid": z.boolean()
});

export const zPreviewFlowRequest = z.strictObject({
  "flow": zFlowQuery
});

export const zRouteDecisionAction = z.enum(["route","reject","invalid"]);

export const zRouteDecision = z.strictObject({
  "action": zRouteDecisionAction,
  "exit_node_id": z.string().nullable(),
  "path": z.array(z.string()),
  "reason": z.string(),
  "rule_id": z.string().nullable()
});

export const zPreviewFlowResponse = z.strictObject({
  "authorization": zFlowAuthorizationDecision,
  "diagnostics": z.array(zPolicyDiagnostic),
  "execution_reason": z.string(),
  "execution_supported": z.boolean(),
  "revision": z.int().gte(1).lte(4294967295),
  "routing": zRouteDecision,
  "valid": z.boolean()
});

export const zPreviewPolicyRequest = z.strictObject({
  "flow": zFlowQuery,
  "policy": zRoutingPolicy
});

export const zPreviewPolicyResponse = z.strictObject({
  "decision": zRouteDecision,
  "diagnostics": z.array(zPolicyDiagnostic),
  "revision": z.int().gte(1).lte(4294967295),
  "valid": z.boolean()
});

export const zPublishConnectionIdentityRequest = z.strictObject({
  "capabilities": z.array(zConnectionCapability).max(3),
  "public_key": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u"))
});

export const zPublishConnectionIdentityResponse = z.strictObject({
  "capabilities": z.array(zConnectionCapability),
  "node_id": z.string(),
  "public_key": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "revision": z.int().gte(1).lte(4294967295)
});

export const zReportConnectionRuntimeRequest = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "epoch": z.int().gte(1).lte(4294967295),
  "listeners": z.array(zConnectionListenerObservation).max(128),
  "peers": z.array(zConnectionNeighborObservation).max(256),
  "reason": zConnectionRuntimeReason,
  "revision": z.int().gte(1).lte(4294967295),
  "sequence": z.int().gte(1).lte(4294967295),
  "state": zConnectionRuntimeState
});

export const zReportConnectionRuntimeResponse = z.strictObject({
  "epoch": z.int().gte(1).lte(4294967295),
  "reported_at": z.iso.datetime({ offset: true }),
  "sequence": z.int().gte(1).lte(4294967295)
});

export const zReportEndpointDiscoveryRequest = z.strictObject({
  "digest": z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u")),
  "epoch": z.int().gte(1).lte(4294967295),
  "revision": z.int().gte(1).lte(4294967295),
  "sequence": z.int().gte(1).lte(4294967295),
  "sockets": z.array(zUdpSocketDiscovery).max(128),
  "state": zEndpointDiscoveryState
});

export const zReportEndpointDiscoveryResponse = z.strictObject({
  "epoch": z.int().gte(1).lte(4294967295),
  "reported_at": z.iso.datetime({ offset: true }),
  "sequence": z.int().gte(1).lte(4294967295)
});

export const zRevokeNodeRequest = z.strictObject({
  "node_id": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points")
});

export const zRevokeNodeResponse = z.strictObject({
  "node": zNodeSummary
});

export const zSaveConnectionsRequest = z.strictObject({
  "configuration": zConnectionPlan,
  "expected_revision": z.int().gte(1).lte(4294967295)
});

export const zSaveMeshAccessRequest = z.strictObject({
  "expected_revision": z.int().gte(1).lte(4294967295),
  "policy": zMeshAccessPolicy,
  "source_jsonc": z.string().nullable().optional()
});

export const zSaveNetworkRequest = z.strictObject({
  "dns": zMeshDnsConfiguration,
  "expected_revision": z.int().gte(1).lte(4294967295),
  "name": z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 128, "maxLength 128 Unicode code points").regex(new RegExp("[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]", "u"))
});

export const zSavePolicyRequest = z.strictObject({
  "expected_revision": z.int().gte(1).lte(4294967295),
  "policy": zRoutingPolicy,
  "source_jsonc": z.string().nullable().optional()
});

export const zTagDefinition = z.strictObject({
  "description": z.string().refine(value => [...value].length <= 256, "maxLength 256 Unicode code points"),
  "tag": z.string().regex(new RegExp("^tag:[a-z0-9][a-z0-9_-]{0,47}$", "u"))
});

export const zSaveTagsRequest = z.strictObject({
  "expected_revision": z.int().gte(1).lte(4294967295),
  "tags": z.array(zTagDefinition).max(64)
});

export const zSetNodeCountryRequest = z.strictObject({
  "country": z.string().nullable(),
  "expected_revision": z.int().gte(1).lte(4294967295),
  "node_id": z.string()
});

export const zSetNodeTagsRequest = z.strictObject({
  "expected_revision": z.int().gte(1).lte(4294967295),
  "node_id": z.string(),
  "tags": z.array(z.string()).max(32)
});

export const zTagSnapshot = z.strictObject({
  "assignments": z.array(zNodeTagAssignment),
  "revision": z.int().gte(1).lte(4294967295),
  "tags": z.array(zTagDefinition).max(64)
});
