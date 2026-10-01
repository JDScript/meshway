// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

export const schemaDocument = {
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "components": {
    "schemas": {
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
            "maxItems": 16,
            "minItems": 1
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
            ]
          },
          "domain": {
            "type": [
              "string",
              "null"
            ]
          },
          "source_node_id": {
            "type": "string"
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
      "MeshAccessSnapshot": {
        "type": "object",
        "required": [
          "revision",
          "policy",
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
          "valid": {
            "type": "boolean"
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
        "description": "One coherent global revision, projected for the authenticated node. Identities\ninclude E2E peers and confer no carrier adjacency. Invalid access withdraws\nevery effective grant. More than 64 union identities rejects the request.",
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
            "$ref": "#/components/schemas/MeshAccessPolicy"
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
            ]
          },
          "kind": {
            "$ref": "#/components/schemas/RouteActionKind"
          },
          "via": {
            "type": "array",
            "items": {
              "type": "string"
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
            "type": "string"
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
              "type": "string"
            }
          }
        },
        "additionalProperties": false
      }
    }
  }
} as const;

export const componentSchemas = schemaDocument.components.schemas;

export function schemaFor(name: keyof typeof componentSchemas) {
  return { ...schemaDocument, $ref: `#/components/schemas/${name}` } as const;
}
