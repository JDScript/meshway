// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

export const schemaDocument = {
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "components": {
    "schemas": {
      "AccessAction": {
        "type": "string",
        "enum": [
          "allow",
          "reject"
        ]
      },
      "AcknowledgeAgentConnectionsRequest": {
        "type": "object",
        "required": [
          "revision",
          "digest"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "AcknowledgeAgentConnectionsResponse": {
        "type": "object",
        "required": [
          "revision",
          "digest",
          "status"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "status": {
            "$ref": "#/components/schemas/ConnectionReceiptStatus"
          }
        },
        "additionalProperties": false
      },
      "AcknowledgeConfigurationRequest": {
        "type": "object",
        "required": [
          "revision",
          "digest"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "AcknowledgeConfigurationResponse": {
        "type": "object",
        "required": [
          "revision",
          "digest"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "AgentConnectionsSnapshot": {
        "type": "object",
        "required": [
          "digest",
          "payload"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "payload": {
            "$ref": "#/components/schemas/NodeConnections"
          }
        },
        "additionalProperties": false
      },
      "AgentDataplaneSnapshot": {
        "type": "object",
        "required": [
          "digest",
          "payload"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "payload": {
            "$ref": "#/components/schemas/NodeDataplane"
          }
        },
        "additionalProperties": false
      },
      "AgentDnsSnapshot": {
        "type": "object",
        "required": [
          "digest",
          "payload"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "payload": {
            "$ref": "#/components/schemas/NodeDnsConfiguration"
          }
        },
        "additionalProperties": false
      },
      "AgentManagedConnectionsSnapshot": {
        "type": "object",
        "required": [
          "network_id",
          "connections",
          "materials",
          "digest"
        ],
        "properties": {
          "connections": {
            "$ref": "#/components/schemas/AgentConnectionsSnapshot"
          },
          "digest": {
            "type": "string"
          },
          "materials": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ManagedVlessMaterial"
            },
            "maxItems": 128
          },
          "network_id": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "AgentManagedDataplaneSnapshot": {
        "type": "object",
        "required": [
          "network_id",
          "dataplane",
          "materials",
          "digest"
        ],
        "properties": {
          "dataplane": {
            "$ref": "#/components/schemas/AgentDataplaneSnapshot"
          },
          "digest": {
            "type": "string"
          },
          "materials": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ManagedVlessMaterial"
            },
            "maxItems": 128
          },
          "network_id": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "AgentNodeResponse": {
        "type": "object",
        "description": "Also lets a pending local enrollment recover its server-assigned node ID.",
        "required": [
          "node"
        ],
        "properties": {
          "node": {
            "$ref": "#/components/schemas/NodeSummary"
          }
        },
        "additionalProperties": false
      },
      "AuthorizeDeviceRequest": {
        "type": "object",
        "required": [
          "redirect_uri",
          "code_challenge",
          "name",
          "agent_version"
        ],
        "properties": {
          "agent_version": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          },
          "code_challenge": {
            "type": "string",
            "maxLength": 43,
            "minLength": 43,
            "pattern": "^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$"
          },
          "name": {
            "type": "string",
            "maxLength": 128,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          },
          "redirect_uri": {
            "type": "string",
            "maxLength": 25,
            "minLength": 25,
            "pattern": "^meshway://enroll/callback$"
          }
        },
        "additionalProperties": false
      },
      "AuthorizeDeviceResponse": {
        "type": "object",
        "required": [
          "code",
          "expires_at"
        ],
        "properties": {
          "code": {
            "type": "string"
          },
          "expires_at": {
            "type": "string",
            "format": "date-time"
          }
        },
        "additionalProperties": false
      },
      "BeginConnectionRuntimeRequest": {
        "type": "object",
        "required": [
          "instance_id",
          "expected_epoch"
        ],
        "properties": {
          "expected_epoch": {
            "type": "integer",
            "format": "int64",
            "description": "Pin the initial epoch read across retries; never refresh after a conflict.",
            "maximum": 4294967295,
            "minimum": 0
          },
          "instance_id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          }
        },
        "additionalProperties": false
      },
      "BeginConnectionRuntimeResponse": {
        "type": "object",
        "required": [
          "epoch"
        ],
        "properties": {
          "epoch": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "ConfigurationSnapshot": {
        "type": "object",
        "required": [
          "digest",
          "payload"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "payload": {
            "$ref": "#/components/schemas/NodeConfiguration"
          }
        },
        "additionalProperties": false
      },
      "ConnectionCandidate": {
        "type": "object",
        "required": [
          "id",
          "dialer_node_id",
          "listener_id",
          "endpoint",
          "priority",
          "credential_ref"
        ],
        "properties": {
          "credential_ref": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "dialer_node_id": {
            "type": "string"
          },
          "endpoint": {
            "$ref": "#/components/schemas/ConnectionEndpoint"
          },
          "id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "listener_id": {
            "type": "string"
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "maximum": 65535,
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "ConnectionCapability": {
        "type": "object",
        "required": [
          "transport",
          "can_dial",
          "can_accept",
          "semantics"
        ],
        "properties": {
          "can_accept": {
            "type": "boolean"
          },
          "can_dial": {
            "type": "boolean"
          },
          "semantics": {
            "$ref": "#/components/schemas/ConnectionSemantics"
          },
          "transport": {
            "$ref": "#/components/schemas/ConnectionTransport"
          }
        },
        "additionalProperties": false
      },
      "ConnectionDiagnostic": {
        "type": "object",
        "required": [
          "item_id",
          "field",
          "message"
        ],
        "properties": {
          "field": {
            "type": "string"
          },
          "item_id": {
            "type": [
              "string",
              "null"
            ]
          },
          "message": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "ConnectionEndpoint": {
        "type": "object",
        "required": [
          "host",
          "port"
        ],
        "properties": {
          "host": {
            "type": "string",
            "maxLength": 253,
            "minLength": 1
          },
          "port": {
            "type": "integer",
            "format": "int32",
            "maximum": 65535,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "ConnectionListener": {
        "type": "object",
        "required": [
          "id",
          "node_id",
          "transport",
          "bind",
          "advertised_endpoints",
          "credential_ref"
        ],
        "properties": {
          "advertised_endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionEndpoint"
            },
            "maxItems": 16
          },
          "bind": {
            "type": "string"
          },
          "credential_ref": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "node_id": {
            "type": "string"
          },
          "transport": {
            "$ref": "#/components/schemas/ConnectionTransport"
          }
        },
        "additionalProperties": false
      },
      "ConnectionListenerObservation": {
        "type": "object",
        "required": [
          "listener_id",
          "state",
          "reason"
        ],
        "properties": {
          "listener_id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "reason": {
            "$ref": "#/components/schemas/ConnectionRuntimeReason"
          },
          "state": {
            "$ref": "#/components/schemas/ConnectionListenerState"
          }
        },
        "additionalProperties": false
      },
      "ConnectionListenerState": {
        "type": "string",
        "enum": [
          "bound",
          "error"
        ]
      },
      "ConnectionNeighborObservation": {
        "type": "object",
        "required": [
          "peer_node_id",
          "transport",
          "initiator_node_id",
          "listener_id",
          "candidate_id",
          "state",
          "reason",
          "probe_fresh",
          "probe_rtt_ms"
        ],
        "properties": {
          "candidate_id": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "initiator_node_id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "listener_id": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "peer_node_id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "probe_fresh": {
            "type": "boolean",
            "description": "A recent authenticated control-nonce probe, not merely a control heartbeat."
          },
          "probe_rtt_ms": {
            "type": [
              "integer",
              "null"
            ],
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          },
          "reason": {
            "$ref": "#/components/schemas/ConnectionRuntimeReason",
            "description": "Sanitized peer-local outcome, independent of global reconcile state."
          },
          "state": {
            "$ref": "#/components/schemas/ConnectionNeighborState"
          },
          "transport": {
            "$ref": "#/components/schemas/ConnectionTransport"
          }
        },
        "additionalProperties": false
      },
      "ConnectionNeighborState": {
        "type": "string",
        "enum": [
          "authenticated",
          "waiting",
          "unreachable"
        ]
      },
      "ConnectionNode": {
        "type": "object",
        "required": [
          "node",
          "public_key",
          "capabilities",
          "acknowledged_revision",
          "acknowledged_digest",
          "acknowledged_at"
        ],
        "properties": {
          "acknowledged_at": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time"
          },
          "acknowledged_digest": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "acknowledged_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          },
          "capabilities": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionCapability"
            }
          },
          "node": {
            "$ref": "#/components/schemas/NodeSummary"
          },
          "public_key": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          }
        },
        "additionalProperties": false
      },
      "ConnectionPeer": {
        "type": "object",
        "required": [
          "node_id",
          "public_key"
        ],
        "properties": {
          "node_id": {
            "type": "string"
          },
          "public_key": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          }
        },
        "additionalProperties": false
      },
      "ConnectionPlan": {
        "type": "object",
        "required": [
          "listeners",
          "candidates"
        ],
        "properties": {
          "candidates": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionCandidate"
            },
            "maxItems": 256
          },
          "listeners": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionListener"
            },
            "maxItems": 128
          }
        },
        "additionalProperties": false
      },
      "ConnectionReceiptStatus": {
        "type": "string",
        "enum": [
          "saved"
        ]
      },
      "ConnectionRuntimeObservation": {
        "type": "object",
        "required": [
          "node_id",
          "epoch",
          "sequence",
          "revision",
          "digest",
          "state",
          "reason",
          "listeners",
          "peers",
          "reported_at",
          "stale_after_seconds",
          "current",
          "stale"
        ],
        "properties": {
          "current": {
            "type": "boolean",
            "description": "Matches the current node-scoped revision/digest and node is active."
          },
          "digest": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "epoch": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "listeners": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionListenerObservation"
            },
            "maxItems": 128
          },
          "node_id": {
            "type": "string"
          },
          "peers": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionNeighborObservation"
            },
            "maxItems": 256
          },
          "reason": {
            "$ref": "#/components/schemas/ConnectionRuntimeReason"
          },
          "reported_at": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time"
          },
          "revision": {
            "type": [
              "integer",
              "null"
            ],
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "sequence": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          },
          "stale": {
            "type": "boolean",
            "description": "Not current, never reported, or receive age >= stale_after_seconds."
          },
          "stale_after_seconds": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "state": {
            "$ref": "#/components/schemas/ConnectionRuntimeState"
          }
        },
        "additionalProperties": false
      },
      "ConnectionRuntimeReason": {
        "type": "string",
        "enum": [
          "none",
          "disabled",
          "invalid_configuration",
          "profile_unavailable",
          "bind_failed",
          "resolve_failed",
          "transport_failed",
          "authentication_failed",
          "probe_failed",
          "lease_superseded",
          "lease_expired",
          "shutting_down",
          "internal_error"
        ]
      },
      "ConnectionRuntimeState": {
        "type": "string",
        "enum": [
          "awaiting_report",
          "disabled",
          "applying",
          "applied",
          "error",
          "withdrawn"
        ]
      },
      "ConnectionRuntimesSnapshot": {
        "type": "object",
        "required": [
          "nodes"
        ],
        "properties": {
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionRuntimeObservation"
            }
          }
        },
        "additionalProperties": false
      },
      "ConnectionSemantics": {
        "type": "string",
        "enum": [
          "datagram",
          "stream"
        ]
      },
      "ConnectionTransport": {
        "type": "string",
        "enum": [
          "native_udp",
          "native_tcp",
          "vless"
        ]
      },
      "ConnectionsSnapshot": {
        "type": "object",
        "required": [
          "revision",
          "configuration",
          "valid",
          "diagnostics",
          "nodes"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConnectionPlan"
          },
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionDiagnostic"
            }
          },
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionNode"
            }
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "valid": {
            "type": "boolean"
          }
        },
        "additionalProperties": false
      },
      "CreateEnrollmentTokenRequest": {
        "type": "object",
        "required": [
          "expires_in_seconds"
        ],
        "properties": {
          "expires_in_seconds": {
            "type": "integer",
            "format": "int32",
            "maximum": 86400,
            "minimum": 60
          }
        },
        "additionalProperties": false
      },
      "CreateEnrollmentTokenResponse": {
        "type": "object",
        "description": "The token is revealed only once, in this creation response.",
        "required": [
          "token_id",
          "token",
          "expires_at"
        ],
        "properties": {
          "expires_at": {
            "type": "string",
            "format": "date-time"
          },
          "token": {
            "type": "string"
          },
          "token_id": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "CreateManagedEnrollmentTokenRequest": {
        "type": "object",
        "required": [
          "expires_in_seconds",
          "initial_listener"
        ],
        "properties": {
          "expires_in_seconds": {
            "type": "integer",
            "format": "int32",
            "maximum": 86400,
            "minimum": 60
          },
          "initial_listener": {
            "$ref": "#/components/schemas/ManagedListenerSettings",
            "description": "Initial intent only: node_id MUST be empty and is bound by enrollment."
          }
        },
        "additionalProperties": false
      },
      "CreateNetworkRequest": {
        "type": "object",
        "required": [
          "name"
        ],
        "properties": {
          "name": {
            "type": "string",
            "maxLength": 128,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          }
        },
        "additionalProperties": false
      },
      "DefaultRouteAction": {
        "type": "string",
        "enum": [
          "direct",
          "reject"
        ]
      },
      "DestinationScope": {
        "type": "string",
        "enum": [
          "any",
          "mesh",
          "internet"
        ]
      },
      "DestinationSelector": {
        "type": "object",
        "required": [
          "kind",
          "value"
        ],
        "properties": {
          "kind": {
            "$ref": "#/components/schemas/DestinationSelectorKind"
          },
          "value": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "DestinationSelectorKind": {
        "type": "string",
        "enum": [
          "node",
          "domain_suffix",
          "cidr",
          "country"
        ]
      },
      "EndpointDiscoveriesSnapshot": {
        "type": "object",
        "required": [
          "network_id",
          "nodes"
        ],
        "properties": {
          "network_id": {
            "type": "string"
          },
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeEndpointDiscovery"
            }
          }
        },
        "additionalProperties": false
      },
      "EndpointDiscoverySource": {
        "type": "string",
        "enum": [
          "managed_udp_socket_stun"
        ]
      },
      "EndpointDiscoveryState": {
        "type": "string",
        "enum": [
          "awaiting_report",
          "disabled",
          "probing",
          "observed",
          "withdrawn",
          "expired",
          "error"
        ]
      },
      "EnrollAgentRequest": {
        "type": "object",
        "description": "Persist the proposed random credential before sending this request. Retry\nwith the same enrollment token, credential, name and enrollment version.",
        "required": [
          "node_credential",
          "name",
          "agent_version"
        ],
        "properties": {
          "agent_version": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          },
          "name": {
            "type": "string",
            "maxLength": 128,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          },
          "node_credential": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          }
        },
        "additionalProperties": false
      },
      "ErrorResponse": {
        "type": "object",
        "required": [
          "message"
        ],
        "properties": {
          "message": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "ExchangeDeviceRequest": {
        "type": "object",
        "required": [
          "code_verifier",
          "redirect_uri",
          "node_credential",
          "name",
          "agent_version"
        ],
        "properties": {
          "agent_version": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          },
          "code_verifier": {
            "type": "string",
            "maxLength": 128,
            "minLength": 43,
            "pattern": "^[A-Za-z0-9._~-]{43,128}$"
          },
          "name": {
            "type": "string",
            "maxLength": 128,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          },
          "node_credential": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "redirect_uri": {
            "type": "string",
            "maxLength": 25,
            "minLength": 25,
            "pattern": "^meshway://enroll/callback$"
          }
        },
        "additionalProperties": false
      },
      "FlowAuthorizationAction": {
        "type": "string",
        "enum": [
          "allow",
          "reject",
          "invalid"
        ]
      },
      "FlowAuthorizationDecision": {
        "type": "object",
        "required": [
          "action",
          "rule_id",
          "matched_direction",
          "reason"
        ],
        "properties": {
          "action": {
            "$ref": "#/components/schemas/FlowAuthorizationAction"
          },
          "matched_direction": {
            "type": [
              "string",
              "null"
            ],
            "pattern": "^(forward|reverse)$"
          },
          "reason": {
            "type": "string"
          },
          "rule_id": {
            "type": [
              "string",
              "null"
            ]
          }
        },
        "additionalProperties": false
      },
      "FlowQuery": {
        "type": "object",
        "description": "Internet country is a preview hint, not automatic geolocation. Mesh country\nand source country always come from the active-node inventory.",
        "required": [
          "source_node_id",
          "destination_node_id",
          "destination_ip",
          "domain",
          "destination_country"
        ],
        "properties": {
          "destination_country": {
            "type": [
              "string",
              "null"
            ]
          },
          "destination_ip": {
            "type": [
              "string",
              "null"
            ]
          },
          "destination_node_id": {
            "type": [
              "string",
              "null"
            ],
            "x-meshway-ref": "node"
          },
          "domain": {
            "type": [
              "string",
              "null"
            ]
          },
          "source_node_id": {
            "type": "string",
            "x-meshway-ref": "node"
          }
        },
        "additionalProperties": false
      },
      "GetConnectionRuntimeEpochResponse": {
        "type": "object",
        "required": [
          "epoch"
        ],
        "properties": {
          "epoch": {
            "type": "integer",
            "format": "int64",
            "description": "Zero means this node has never acquired a runtime lease.",
            "maximum": 4294967295,
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "HeartbeatRequest": {
        "type": "object",
        "description": "Node identity is derived exclusively from the node bearer credential.",
        "required": [
          "agent_version"
        ],
        "properties": {
          "agent_version": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          }
        },
        "additionalProperties": false
      },
      "IncomingConnectionApproval": {
        "type": "object",
        "required": [
          "candidate_id",
          "listener_id",
          "peer_node_id",
          "transport"
        ],
        "properties": {
          "candidate_id": {
            "type": "string"
          },
          "listener_id": {
            "type": "string"
          },
          "peer_node_id": {
            "type": "string"
          },
          "transport": {
            "$ref": "#/components/schemas/ConnectionTransport"
          }
        },
        "additionalProperties": false
      },
      "ManagedListenerSettings": {
        "type": "object",
        "required": [
          "id",
          "node_id",
          "enabled",
          "transport",
          "bind_address",
          "bind_port",
          "advertised_endpoints",
          "vless"
        ],
        "properties": {
          "advertised_endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionEndpoint"
            },
            "description": "Explicit announcement, not proof of reachability. Empty means no dial endpoint.",
            "maxItems": 16
          },
          "bind_address": {
            "type": "string"
          },
          "bind_port": {
            "type": "integer",
            "format": "int32",
            "maximum": 65535,
            "minimum": 1
          },
          "enabled": {
            "type": "boolean"
          },
          "id": {
            "type": "string",
            "maxLength": 48,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "node_id": {
            "type": "string"
          },
          "transport": {
            "$ref": "#/components/schemas/ConnectionTransport"
          },
          "vless": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ManagedVlessSettings"
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "ManagedListenerSummary": {
        "type": "object",
        "required": [
          "settings",
          "material_version",
          "certificate_sha256",
          "compiled_capability_available"
        ],
        "properties": {
          "certificate_sha256": {
            "type": [
              "string",
              "null"
            ]
          },
          "compiled_capability_available": {
            "type": "boolean",
            "description": "False is retained intent awaiting a compatible identity/capability report."
          },
          "material_version": {
            "type": "integer",
            "format": "int32",
            "maximum": 4294967295,
            "minimum": 1
          },
          "settings": {
            "$ref": "#/components/schemas/ManagedListenerSettings"
          }
        },
        "additionalProperties": false
      },
      "ManagedListenersSnapshot": {
        "type": "object",
        "required": [
          "revision",
          "listeners"
        ],
        "properties": {
          "listeners": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ManagedListenerSummary"
            }
          },
          "revision": {
            "type": "integer",
            "format": "int32",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "ManagedVlessMaterial": {
        "type": "object",
        "description": "No Debug: serialization is permitted only on the node-authenticated endpoint.",
        "required": [
          "listener_id",
          "credential_ref",
          "version",
          "settings",
          "uuid",
          "certificate_pem",
          "certificate_sha256",
          "private_key_pem"
        ],
        "properties": {
          "certificate_pem": {
            "type": "string",
            "maxLength": 32768,
            "minLength": 1
          },
          "certificate_sha256": {
            "type": "string"
          },
          "credential_ref": {
            "type": "string"
          },
          "listener_id": {
            "type": "string"
          },
          "private_key_pem": {
            "type": [
              "string",
              "null"
            ],
            "format": "password",
            "description": "Only listener owner receives this; dialers receive null.",
            "maxLength": 16384,
            "minLength": 1
          },
          "settings": {
            "$ref": "#/components/schemas/ManagedVlessSettings"
          },
          "uuid": {
            "type": "string",
            "format": "password"
          },
          "version": {
            "type": "integer",
            "format": "int32",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "ManagedVlessSettings": {
        "type": "object",
        "required": [
          "stream",
          "server_name",
          "websocket_path",
          "websocket_host"
        ],
        "properties": {
          "server_name": {
            "type": "string",
            "description": "Certificate name and client verification name, independent of HTTP Host.",
            "maxLength": 253,
            "minLength": 1
          },
          "stream": {
            "$ref": "#/components/schemas/VlessStreamTransport"
          },
          "websocket_host": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 253,
            "minLength": 1
          },
          "websocket_path": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 1024,
            "minLength": 1
          }
        },
        "additionalProperties": false
      },
      "MeshAccessGrant": {
        "type": "object",
        "required": [
          "id",
          "path"
        ],
        "properties": {
          "id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "path": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Exact ordered full path, including source and destination. No repeats.",
            "maxItems": 8,
            "minItems": 2
          }
        },
        "additionalProperties": false
      },
      "MeshAccessPolicy": {
        "type": "object",
        "required": [
          "default_action",
          "rules"
        ],
        "properties": {
          "default_action": {
            "$ref": "#/components/schemas/AccessAction"
          },
          "rules": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MeshAccessRule"
            },
            "maxItems": 128
          }
        },
        "additionalProperties": false
      },
      "MeshAccessRule": {
        "type": "object",
        "required": [
          "id",
          "source",
          "destination",
          "action",
          "bidirectional"
        ],
        "properties": {
          "action": {
            "$ref": "#/components/schemas/AccessAction"
          },
          "bidirectional": {
            "type": "boolean"
          },
          "destination": {
            "type": "array",
            "items": {
              "type": "string",
              "x-meshway-ref": "selector"
            },
            "maxItems": 32,
            "minItems": 1
          },
          "id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "source": {
            "type": "array",
            "items": {
              "type": "string",
              "x-meshway-ref": "selector"
            },
            "maxItems": 32,
            "minItems": 1
          }
        },
        "additionalProperties": false
      },
      "MeshAccessSnapshot": {
        "type": "object",
        "required": [
          "revision",
          "policy",
          "source_jsonc",
          "valid",
          "diagnostics"
        ],
        "properties": {
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PolicyDiagnostic"
            }
          },
          "policy": {
            "$ref": "#/components/schemas/MeshAccessPolicy",
            "description": "Saved source intent, retained even when references become invalid."
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "source_jsonc": {
            "type": [
              "string",
              "null"
            ]
          },
          "valid": {
            "type": "boolean"
          }
        },
        "additionalProperties": false
      },
      "MeshDnsConfiguration": {
        "type": "object",
        "required": [
          "enabled",
          "zone",
          "ttl_seconds"
        ],
        "properties": {
          "enabled": {
            "type": "boolean"
          },
          "ttl_seconds": {
            "type": "integer",
            "format": "int32",
            "maximum": 300,
            "minimum": 1
          },
          "zone": {
            "type": "string",
            "maxLength": 128,
            "minLength": 3
          }
        },
        "additionalProperties": false
      },
      "MeshNetwork": {
        "type": "object",
        "required": [
          "ipv4_prefix",
          "ipv6_prefix"
        ],
        "properties": {
          "ipv4_prefix": {
            "type": "string"
          },
          "ipv6_prefix": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "MeshNode": {
        "type": "object",
        "description": "Public active-node inventory; country is administrator-managed metadata.",
        "required": [
          "id",
          "name",
          "country",
          "mesh_ipv4",
          "mesh_ipv6"
        ],
        "properties": {
          "country": {
            "type": [
              "string",
              "null"
            ]
          },
          "id": {
            "type": "string"
          },
          "mesh_ipv4": {
            "type": "string"
          },
          "mesh_ipv6": {
            "type": "string"
          },
          "name": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "MeshPathAuthorization": {
        "type": "object",
        "description": "Internal compiled path authorization. Its JSON remains `{ \"grants\": [...] }`\nso schema-version-1 NodeDataplane remains byte-contract compatible.",
        "required": [
          "grants"
        ],
        "properties": {
          "grants": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MeshAccessGrant"
            },
            "maxItems": 128
          }
        },
        "additionalProperties": false
      },
      "NetworkList": {
        "type": "object",
        "required": [
          "networks"
        ],
        "properties": {
          "networks": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NetworkSnapshot"
            }
          }
        },
        "additionalProperties": false
      },
      "NetworkSnapshot": {
        "type": "object",
        "description": "Saved intent at the network configuration revision. Address pools are immutable.",
        "required": [
          "revision",
          "network_id",
          "name",
          "network",
          "dns"
        ],
        "properties": {
          "dns": {
            "$ref": "#/components/schemas/MeshDnsConfiguration"
          },
          "name": {
            "type": "string",
            "maxLength": 128,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          },
          "network": {
            "$ref": "#/components/schemas/MeshNetwork"
          },
          "network_id": {
            "type": "string",
            "maxLength": 32,
            "minLength": 32,
            "pattern": "^[0-9a-f]{32}$"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "NodeConfiguration": {
        "type": "object",
        "required": [
          "schema_version",
          "revision",
          "network",
          "node",
          "peers",
          "routing_policy",
          "policy_valid",
          "diagnostics"
        ],
        "properties": {
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PolicyDiagnostic"
            }
          },
          "network": {
            "$ref": "#/components/schemas/MeshNetwork"
          },
          "node": {
            "$ref": "#/components/schemas/MeshNode"
          },
          "peers": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MeshNode"
            }
          },
          "policy_valid": {
            "type": "boolean"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "routing_policy": {
            "$ref": "#/components/schemas/RoutingPolicy"
          },
          "schema_version": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "NodeConnections": {
        "type": "object",
        "required": [
          "schema_version",
          "revision",
          "node_id",
          "public_key",
          "capabilities",
          "valid",
          "diagnostics",
          "listeners",
          "outgoing",
          "incoming",
          "peers"
        ],
        "properties": {
          "capabilities": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionCapability"
            }
          },
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionDiagnostic"
            }
          },
          "incoming": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/IncomingConnectionApproval"
            }
          },
          "listeners": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionListener"
            }
          },
          "node_id": {
            "type": "string"
          },
          "outgoing": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/OutgoingConnectionCandidate"
            }
          },
          "peers": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionPeer"
            }
          },
          "public_key": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "schema_version": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "valid": {
            "type": "boolean"
          }
        },
        "additionalProperties": false
      },
      "NodeDataplane": {
        "type": "object",
        "description": "One coherent global revision, projected for the authenticated node.",
        "required": [
          "schema_version",
          "revision",
          "node_id",
          "configuration",
          "connections",
          "access_policy",
          "access_valid",
          "identities"
        ],
        "properties": {
          "access_policy": {
            "$ref": "#/components/schemas/MeshPathAuthorization"
          },
          "access_valid": {
            "type": "boolean"
          },
          "configuration": {
            "$ref": "#/components/schemas/NodeConfiguration"
          },
          "connections": {
            "$ref": "#/components/schemas/NodeConnections"
          },
          "identities": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionPeer"
            },
            "maxItems": 64
          },
          "node_id": {
            "type": "string"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "schema_version": {
            "type": "integer",
            "format": "int64",
            "maximum": 1,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "NodeDnsConfiguration": {
        "type": "object",
        "required": [
          "schema_version",
          "revision",
          "node_id",
          "network_id",
          "dns",
          "records",
          "diagnostics"
        ],
        "properties": {
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PolicyDiagnostic"
            },
            "maxItems": 64
          },
          "dns": {
            "$ref": "#/components/schemas/MeshDnsConfiguration"
          },
          "network_id": {
            "type": "string",
            "maxLength": 32,
            "minLength": 32,
            "pattern": "^[0-9a-f]{32}$"
          },
          "node_id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "records": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeDnsRecord"
            },
            "maxItems": 1024
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "schema_version": {
            "type": "integer",
            "format": "int32",
            "maximum": 1,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "NodeDnsRecord": {
        "type": "object",
        "required": [
          "node_id",
          "canonical_name",
          "aliases",
          "mesh_ipv4",
          "mesh_ipv6"
        ],
        "properties": {
          "aliases": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "maxItems": 1
          },
          "canonical_name": {
            "type": "string",
            "maxLength": 253,
            "minLength": 1,
            "pattern": "^id\\.[0-9a-f]{32}\\.[0-9a-f]{32}\\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$"
          },
          "mesh_ipv4": {
            "type": "string"
          },
          "mesh_ipv6": {
            "type": "string"
          },
          "node_id": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          }
        },
        "additionalProperties": false
      },
      "NodeEndpointDiscovery": {
        "type": "object",
        "required": [
          "node_id",
          "epoch",
          "sequence",
          "revision",
          "digest",
          "state",
          "reported_at",
          "current",
          "stale",
          "sockets"
        ],
        "properties": {
          "current": {
            "type": "boolean"
          },
          "digest": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "epoch": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          },
          "node_id": {
            "type": "string"
          },
          "reported_at": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time"
          },
          "revision": {
            "type": [
              "integer",
              "null"
            ],
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "sequence": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          },
          "sockets": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/UdpSocketDiscovery"
            },
            "maxItems": 128
          },
          "stale": {
            "type": "boolean"
          },
          "state": {
            "$ref": "#/components/schemas/EndpointDiscoveryState"
          }
        },
        "additionalProperties": false
      },
      "NodeList": {
        "type": "object",
        "required": [
          "nodes"
        ],
        "properties": {
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeSummary"
            }
          }
        },
        "additionalProperties": false
      },
      "NodeSummary": {
        "type": "object",
        "description": "Durable control-plane identity. Last contact is not tunnel health.",
        "required": [
          "id",
          "name",
          "mesh_ipv4",
          "mesh_ipv6",
          "country",
          "configuration_revision",
          "acknowledged_revision",
          "agent_version",
          "last_seen",
          "registered_at",
          "revoked_at"
        ],
        "properties": {
          "acknowledged_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          },
          "agent_version": {
            "type": "string"
          },
          "configuration_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "country": {
            "type": [
              "string",
              "null"
            ]
          },
          "id": {
            "type": "string"
          },
          "last_seen": {
            "type": "string",
            "format": "date-time"
          },
          "mesh_ipv4": {
            "type": "string"
          },
          "mesh_ipv6": {
            "type": "string"
          },
          "name": {
            "type": "string"
          },
          "registered_at": {
            "type": "string",
            "format": "date-time"
          },
          "revoked_at": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time"
          }
        },
        "additionalProperties": false
      },
      "NodeTagAssignment": {
        "type": "object",
        "required": [
          "node_id",
          "tags"
        ],
        "properties": {
          "node_id": {
            "type": "string",
            "x-meshway-ref": "node"
          },
          "tags": {
            "type": "array",
            "items": {
              "type": "string",
              "x-meshway-ref": "tag"
            },
            "maxItems": 32
          }
        },
        "additionalProperties": false
      },
      "OutgoingConnectionCandidate": {
        "type": "object",
        "required": [
          "id",
          "listener_id",
          "peer_node_id",
          "transport",
          "endpoint",
          "priority",
          "credential_ref"
        ],
        "properties": {
          "credential_ref": {
            "type": [
              "string",
              "null"
            ]
          },
          "endpoint": {
            "$ref": "#/components/schemas/ConnectionEndpoint"
          },
          "id": {
            "type": "string"
          },
          "listener_id": {
            "type": "string"
          },
          "peer_node_id": {
            "type": "string"
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "maximum": 65535,
            "minimum": 0
          },
          "transport": {
            "$ref": "#/components/schemas/ConnectionTransport"
          }
        },
        "additionalProperties": false
      },
      "Overview": {
        "type": "object",
        "required": [
          "name",
          "version",
          "node_count",
          "configuration_revision"
        ],
        "properties": {
          "configuration_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "name": {
            "type": "string"
          },
          "node_count": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          },
          "version": {
            "type": "string"
          }
        },
        "additionalProperties": false
      },
      "PolicyDiagnostic": {
        "type": "object",
        "required": [
          "rule_id",
          "field",
          "message"
        ],
        "properties": {
          "field": {
            "type": "string"
          },
          "message": {
            "type": "string"
          },
          "rule_id": {
            "type": [
              "string",
              "null"
            ]
          }
        },
        "additionalProperties": false
      },
      "PolicySnapshot": {
        "type": "object",
        "required": [
          "revision",
          "network",
          "policy",
          "source_jsonc",
          "valid",
          "diagnostics"
        ],
        "properties": {
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PolicyDiagnostic"
            }
          },
          "network": {
            "$ref": "#/components/schemas/MeshNetwork"
          },
          "policy": {
            "$ref": "#/components/schemas/RoutingPolicy"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "source_jsonc": {
            "type": [
              "string",
              "null"
            ]
          },
          "valid": {
            "type": "boolean"
          }
        },
        "additionalProperties": false
      },
      "PreviewFlowRequest": {
        "type": "object",
        "required": [
          "flow"
        ],
        "properties": {
          "flow": {
            "$ref": "#/components/schemas/FlowQuery"
          }
        },
        "additionalProperties": false
      },
      "PreviewFlowResponse": {
        "type": "object",
        "required": [
          "revision",
          "valid",
          "authorization",
          "routing",
          "diagnostics",
          "execution_supported",
          "execution_reason"
        ],
        "properties": {
          "authorization": {
            "$ref": "#/components/schemas/FlowAuthorizationDecision"
          },
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PolicyDiagnostic"
            }
          },
          "execution_reason": {
            "type": "string"
          },
          "execution_supported": {
            "type": "boolean"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "routing": {
            "$ref": "#/components/schemas/RouteDecision"
          },
          "valid": {
            "type": "boolean"
          }
        },
        "additionalProperties": false
      },
      "PreviewPolicyRequest": {
        "type": "object",
        "required": [
          "policy",
          "flow"
        ],
        "properties": {
          "flow": {
            "$ref": "#/components/schemas/FlowQuery"
          },
          "policy": {
            "$ref": "#/components/schemas/RoutingPolicy"
          }
        },
        "additionalProperties": false
      },
      "PreviewPolicyResponse": {
        "type": "object",
        "required": [
          "revision",
          "valid",
          "diagnostics",
          "decision"
        ],
        "properties": {
          "decision": {
            "$ref": "#/components/schemas/RouteDecision"
          },
          "diagnostics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PolicyDiagnostic"
            }
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "valid": {
            "type": "boolean"
          }
        },
        "additionalProperties": false
      },
      "PublishConnectionIdentityRequest": {
        "type": "object",
        "required": [
          "public_key",
          "capabilities"
        ],
        "properties": {
          "capabilities": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionCapability"
            },
            "maxItems": 3
          },
          "public_key": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          }
        },
        "additionalProperties": false
      },
      "PublishConnectionIdentityResponse": {
        "type": "object",
        "required": [
          "revision",
          "node_id",
          "public_key",
          "capabilities"
        ],
        "properties": {
          "capabilities": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionCapability"
            }
          },
          "node_id": {
            "type": "string"
          },
          "public_key": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "PublishManagedTransportCapabilitiesRequest": {
        "type": "object",
        "required": [
          "vless_streams"
        ],
        "properties": {
          "vless_streams": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/VlessStreamTransport"
            },
            "maxItems": 2
          }
        },
        "additionalProperties": false
      },
      "PublishManagedTransportCapabilitiesResponse": {
        "type": "object",
        "required": [
          "revision"
        ],
        "properties": {
          "revision": {
            "type": "integer",
            "format": "int32",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "ReportConnectionRuntimeRequest": {
        "type": "object",
        "required": [
          "epoch",
          "sequence",
          "revision",
          "digest",
          "state",
          "reason",
          "listeners",
          "peers"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "epoch": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "listeners": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionListenerObservation"
            },
            "maxItems": 128
          },
          "peers": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConnectionNeighborObservation"
            },
            "maxItems": 256
          },
          "reason": {
            "$ref": "#/components/schemas/ConnectionRuntimeReason"
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "sequence": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "state": {
            "$ref": "#/components/schemas/ConnectionRuntimeState"
          }
        },
        "additionalProperties": false
      },
      "ReportConnectionRuntimeResponse": {
        "type": "object",
        "required": [
          "epoch",
          "sequence",
          "reported_at"
        ],
        "properties": {
          "epoch": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "reported_at": {
            "type": "string",
            "format": "date-time"
          },
          "sequence": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "ReportEndpointDiscoveryRequest": {
        "type": "object",
        "required": [
          "epoch",
          "sequence",
          "revision",
          "digest",
          "state",
          "sockets"
        ],
        "properties": {
          "digest": {
            "type": "string",
            "maxLength": 64,
            "minLength": 64,
            "pattern": "^[0-9a-f]{64}$"
          },
          "epoch": {
            "type": "integer",
            "format": "int64",
            "description": "Reuses the authenticated managed runtime's acquired epoch, not a new identity.",
            "maximum": 4294967295,
            "minimum": 1
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "sequence": {
            "type": "integer",
            "format": "int64",
            "description": "Independent monotonic discovery report sequence within that epoch.",
            "maximum": 4294967295,
            "minimum": 1
          },
          "sockets": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/UdpSocketDiscovery"
            },
            "maxItems": 128
          },
          "state": {
            "$ref": "#/components/schemas/EndpointDiscoveryState"
          }
        },
        "additionalProperties": false
      },
      "ReportEndpointDiscoveryResponse": {
        "type": "object",
        "required": [
          "epoch",
          "sequence",
          "reported_at"
        ],
        "properties": {
          "epoch": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "reported_at": {
            "type": "string",
            "format": "date-time"
          },
          "sequence": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "RevokeNodeRequest": {
        "type": "object",
        "required": [
          "node_id"
        ],
        "properties": {
          "node_id": {
            "type": "string",
            "maxLength": 128,
            "minLength": 1
          }
        },
        "additionalProperties": false
      },
      "RevokeNodeResponse": {
        "type": "object",
        "required": [
          "node"
        ],
        "properties": {
          "node": {
            "$ref": "#/components/schemas/NodeSummary"
          }
        },
        "additionalProperties": false
      },
      "RouteAction": {
        "type": "object",
        "required": [
          "kind",
          "via",
          "exit_node_id"
        ],
        "properties": {
          "exit_node_id": {
            "type": [
              "string",
              "null"
            ],
            "x-meshway-ref": "node"
          },
          "kind": {
            "$ref": "#/components/schemas/RouteActionKind"
          },
          "via": {
            "type": "array",
            "items": {
              "type": "string",
              "x-meshway-ref": "node"
            }
          }
        },
        "additionalProperties": false
      },
      "RouteActionKind": {
        "type": "string",
        "enum": [
          "route",
          "reject"
        ]
      },
      "RouteDecision": {
        "type": "object",
        "required": [
          "action",
          "rule_id",
          "path",
          "exit_node_id",
          "reason"
        ],
        "properties": {
          "action": {
            "$ref": "#/components/schemas/RouteDecisionAction"
          },
          "exit_node_id": {
            "type": [
              "string",
              "null"
            ]
          },
          "path": {
            "type": "array",
            "items": {
              "type": "string"
            }
          },
          "reason": {
            "type": "string"
          },
          "rule_id": {
            "type": [
              "string",
              "null"
            ]
          }
        },
        "additionalProperties": false
      },
      "RouteDecisionAction": {
        "type": "string",
        "enum": [
          "route",
          "reject",
          "invalid"
        ]
      },
      "RouteRule": {
        "type": "object",
        "required": [
          "id",
          "source",
          "destination_scope",
          "destination_any",
          "action"
        ],
        "properties": {
          "action": {
            "$ref": "#/components/schemas/RouteAction"
          },
          "destination_any": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/DestinationSelector"
            }
          },
          "destination_scope": {
            "$ref": "#/components/schemas/DestinationScope"
          },
          "id": {
            "type": "string"
          },
          "source": {
            "$ref": "#/components/schemas/SourceSelector"
          }
        },
        "additionalProperties": false
      },
      "RoutingPolicy": {
        "type": "object",
        "required": [
          "default_action",
          "rules"
        ],
        "properties": {
          "default_action": {
            "$ref": "#/components/schemas/DefaultRouteAction"
          },
          "rules": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/RouteRule"
            }
          }
        },
        "additionalProperties": false
      },
      "SaveConnectionsRequest": {
        "type": "object",
        "required": [
          "expected_revision",
          "configuration"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConnectionPlan"
          },
          "expected_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          }
        },
        "additionalProperties": false
      },
      "SaveManagedListenerRequest": {
        "type": "object",
        "required": [
          "expected_revision",
          "settings"
        ],
        "properties": {
          "expected_revision": {
            "type": "integer",
            "format": "int32",
            "maximum": 4294967295,
            "minimum": 1
          },
          "settings": {
            "$ref": "#/components/schemas/ManagedListenerSettings"
          }
        },
        "additionalProperties": false
      },
      "SaveMeshAccessRequest": {
        "type": "object",
        "required": [
          "expected_revision",
          "policy"
        ],
        "properties": {
          "expected_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "policy": {
            "$ref": "#/components/schemas/MeshAccessPolicy"
          },
          "source_jsonc": {
            "type": [
              "string",
              "null"
            ]
          }
        },
        "additionalProperties": false
      },
      "SaveNetworkRequest": {
        "type": "object",
        "required": [
          "expected_revision",
          "name",
          "dns"
        ],
        "properties": {
          "dns": {
            "$ref": "#/components/schemas/MeshDnsConfiguration"
          },
          "expected_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "name": {
            "type": "string",
            "maxLength": 128,
            "minLength": 1,
            "pattern": "[^\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]"
          }
        },
        "additionalProperties": false
      },
      "SavePolicyRequest": {
        "type": "object",
        "required": [
          "expected_revision",
          "policy"
        ],
        "properties": {
          "expected_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "policy": {
            "$ref": "#/components/schemas/RoutingPolicy"
          },
          "source_jsonc": {
            "type": [
              "string",
              "null"
            ]
          }
        },
        "additionalProperties": false
      },
      "SaveTagsRequest": {
        "type": "object",
        "required": [
          "expected_revision",
          "tags"
        ],
        "properties": {
          "expected_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "tags": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TagDefinition"
            },
            "maxItems": 64
          }
        },
        "additionalProperties": false
      },
      "SetNodeCountryRequest": {
        "type": "object",
        "required": [
          "node_id",
          "country",
          "expected_revision"
        ],
        "properties": {
          "country": {
            "type": [
              "string",
              "null"
            ]
          },
          "expected_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "node_id": {
            "type": "string",
            "x-meshway-ref": "node"
          }
        },
        "additionalProperties": false
      },
      "SetNodeTagsRequest": {
        "type": "object",
        "required": [
          "expected_revision",
          "node_id",
          "tags"
        ],
        "properties": {
          "expected_revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "node_id": {
            "type": "string",
            "x-meshway-ref": "node"
          },
          "tags": {
            "type": "array",
            "items": {
              "type": "string",
              "x-meshway-ref": "tag"
            },
            "maxItems": 32
          }
        },
        "additionalProperties": false
      },
      "SourceSelector": {
        "type": "object",
        "description": "Empty dimensions are wildcards; dimensions combine with AND, members with OR.",
        "required": [
          "node_ids",
          "countries"
        ],
        "properties": {
          "countries": {
            "type": "array",
            "items": {
              "type": "string"
            }
          },
          "node_ids": {
            "type": "array",
            "items": {
              "type": "string",
              "x-meshway-ref": "node"
            }
          }
        },
        "additionalProperties": false
      },
      "StunDiscoveryOutcome": {
        "type": "string",
        "enum": [
          "mapped",
          "timeout",
          "invalid_response",
          "io_error",
          "stun_error"
        ]
      },
      "StunEndpointObservation": {
        "type": "object",
        "required": [
          "observer",
          "outcome",
          "mapped_endpoint",
          "rtt_ms",
          "observed_at",
          "expires_at"
        ],
        "properties": {
          "expires_at": {
            "type": "string",
            "format": "date-time"
          },
          "mapped_endpoint": {
            "type": [
              "string",
              "null"
            ],
            "maxLength": 64,
            "minLength": 3
          },
          "observed_at": {
            "type": "string",
            "format": "date-time",
            "description": "Repeated reports preserve these times; retransmission is not renewal."
          },
          "observer": {
            "type": "string",
            "description": "Explicit numeric STUN observer endpoint, not a Meshway peer identity.",
            "maxLength": 64,
            "minLength": 3
          },
          "outcome": {
            "$ref": "#/components/schemas/StunDiscoveryOutcome"
          },
          "rtt_ms": {
            "type": [
              "integer",
              "null"
            ],
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "TagDefinition": {
        "type": "object",
        "required": [
          "tag",
          "description"
        ],
        "properties": {
          "description": {
            "type": "string",
            "maxLength": 256
          },
          "tag": {
            "type": "string",
            "pattern": "^tag:[a-z0-9][a-z0-9_-]{0,47}$",
            "x-meshway-ref": "tag"
          }
        },
        "additionalProperties": false
      },
      "TagSnapshot": {
        "type": "object",
        "required": [
          "revision",
          "tags",
          "assignments"
        ],
        "properties": {
          "assignments": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeTagAssignment"
            }
          },
          "revision": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "tags": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TagDefinition"
            },
            "maxItems": 64
          }
        },
        "additionalProperties": false
      },
      "UdpDiscoveryResourceKind": {
        "type": "string",
        "enum": [
          "listener",
          "outgoing_candidate"
        ]
      },
      "UdpSocketDiscovery": {
        "type": "object",
        "required": [
          "resource_kind",
          "resource_id",
          "generation",
          "socket_id",
          "bound_address",
          "source",
          "observations"
        ],
        "properties": {
          "bound_address": {
            "type": "string",
            "maxLength": 64,
            "minLength": 3
          },
          "generation": {
            "type": "integer",
            "format": "int64",
            "description": "Public process-local runtime generation and socket ordinal.",
            "maximum": 4294967295,
            "minimum": 1
          },
          "observations": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/StunEndpointObservation"
            },
            "maxItems": 4
          },
          "resource_id": {
            "type": "string",
            "description": "Controller listener/candidate ID, never a private runtime token.",
            "maxLength": 64,
            "minLength": 1,
            "pattern": "^[A-Za-z0-9._-]+$"
          },
          "resource_kind": {
            "$ref": "#/components/schemas/UdpDiscoveryResourceKind"
          },
          "socket_id": {
            "type": "integer",
            "format": "int64",
            "maximum": 4294967295,
            "minimum": 1
          },
          "source": {
            "$ref": "#/components/schemas/EndpointDiscoverySource"
          }
        },
        "additionalProperties": false
      },
      "VlessStreamTransport": {
        "type": "string",
        "enum": [
          "tcp",
          "websocket"
        ]
      }
    }
  }
} as const;

export const componentSchemas = schemaDocument.components.schemas;

export function schemaFor(name: keyof typeof componentSchemas) {
  return { ...schemaDocument, $ref: `#/components/schemas/${name}` } as const;
}
