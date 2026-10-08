// Generated from the Server OpenAPI by scripts/protocol/generate.mjs. Do not edit.

export const schemaDocument = {
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "components": {
    "schemas": {
      "AcceptUdpTraversalResponse": {
        "oneOf": [
          {
            "type": "object",
            "description": "The caller accepted and the other participant has not yet accepted.",
            "required": [
              "offer",
              "status"
            ],
            "properties": {
              "offer": {
                "$ref": "#/components/schemas/UdpTraversalOffer",
                "description": "Unchanged offer awaiting the peer's acceptance before its deadline."
              },
              "status": {
                "type": "string",
                "enum": [
                  "waiting"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Both participants accepted, allowing bounded acquisition under the original offer.",
            "required": [
              "offer",
              "status"
            ],
            "properties": {
              "offer": {
                "$ref": "#/components/schemas/UdpTraversalOffer",
                "description": "Unchanged offer whose bindings and deadline constrain acquisition probes."
              },
              "status": {
                "type": "string",
                "enum": [
                  "active"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The offer cannot be accepted under its current bindings or lifecycle.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/UdpTraversalUnavailableReason",
                "description": "Closed coordination condition that prevented acceptance."
              },
              "status": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Server result of accepting the exact offer after the caller reserves its actual UDP receiver slot."
      },
      "AccessDefault": {
        "type": "string",
        "description": "Behavior for Mesh business traffic without an explicit access grant.",
        "enum": [
          "deny"
        ]
      },
      "AccessEvaluation": {
        "type": "object",
        "description": "Whether and by which rules Access allows traffic from the source to the destination.",
        "required": [
          "allowed",
          "grants"
        ],
        "properties": {
          "allowed": {
            "type": "boolean",
            "description": "Some grant authorizes the destination independently of resolution."
          },
          "grants": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/AccessGrantEvaluation"
            }
          }
        },
        "additionalProperties": false
      },
      "AccessGrantEvaluation": {
        "type": "object",
        "description": "One Access rule that covers the destination for the source.",
        "required": [
          "rule",
          "traffic",
          "reverse",
          "resolutionDependent"
        ],
        "properties": {
          "resolutionDependent": {
            "type": "boolean",
            "description": "Authorizes only for some resolutions: an address grant for a domain, a\ndomain grant for an address."
          },
          "reverse": {
            "type": "boolean",
            "description": "Matched through `bidirectional` from the destination side."
          },
          "rule": {
            "type": "string",
            "description": "`/access/<index>`."
          },
          "traffic": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "The rule's `ip` entries: protocol/port grants such as `*`, `tcp:443`, `udp:*`."
          }
        },
        "additionalProperties": false
      },
      "AccessRule": {
        "type": "object",
        "description": "An additive business traffic grant; all matching grants are combined.",
        "required": [
          "src",
          "dst",
          "ip"
        ],
        "properties": {
          "bidirectional": {
            "type": "boolean",
            "description": "Allow reverse initiation as well; defaults to false and requires both sides to select only Mesh nodes."
          },
          "dst": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Nonempty destination selectors: Mesh selectors, canonical CIDRs, `domain:<suffix>` or public-only `autogroup:internet`."
          },
          "ip": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Nonempty protocol/port grants such as `*`, `tcp:443`, `tcp:8000-8100`, `udp:*` or `icmp:*`."
          },
          "remark": {
            "type": "string",
            "description": "Free-form description, defaulting to an empty string; it is not a rule identifier."
          },
          "src": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Nonempty source selectors: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`."
          }
        },
        "additionalProperties": false
      },
      "AccountResponse": {
        "type": "object",
        "description": "The authenticated user's account metadata.",
        "required": [
          "requestId",
          "user"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "user": {
            "$ref": "#/components/schemas/UserView",
            "description": "Current account identifier and canonical email address."
          }
        },
        "additionalProperties": false
      },
      "AccountSessionView": {
        "type": "object",
        "description": "Non-secret metadata for one of the authenticated user's active sessions.",
        "required": [
          "id",
          "createdAt",
          "expiresAt",
          "lastUsedAt",
          "clientLabel",
          "current",
          "delegationClient"
        ],
        "properties": {
          "clientLabel": {
            "type": [
              "string",
              "null"
            ],
            "description": "Coarse self-reported client label, or null when the User-Agent is absent or unrecognized."
          },
          "createdAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which this session was issued."
          },
          "current": {
            "type": "boolean",
            "description": "Whether this session authenticated the current request."
          },
          "delegationClient": {
            "type": [
              "string",
              "null"
            ],
            "description": "The delegation client this session was issued to, or null for a session the user signed in with."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "Absolute session expiry, not extended by session use."
          },
          "id": {
            "$ref": "#/components/schemas/SessionId",
            "description": "Stable identifier used to revoke this session."
          },
          "lastUsedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Last recorded authenticated use, with writes coalesced over 30 seconds; null before first use."
          }
        },
        "additionalProperties": false
      },
      "AccountSessionsResponse": {
        "type": "object",
        "description": "A bounded page of the authenticated user's active sessions.",
        "required": [
          "requestId",
          "items",
          "nextCursor",
          "asOf"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the initial pagination window was established."
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/AccountSessionView"
            },
            "description": "Active sessions in descending creation order within the initial query bound."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Opaque continuation cursor, or null when no further page is available."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "AcquireInstanceRequest": {
        "type": "object",
        "description": "Authenticated Node request to acquire its runtime instance with actual build capabilities.",
        "required": [
          "instanceId",
          "capabilities"
        ],
        "properties": {
          "capabilities": {
            "type": "object",
            "description": "Actual supported capabilities fixed by authenticated acquisition rather than ordinary State reports.",
            "additionalProperties": {
              "$ref": "#/components/schemas/Capability"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Node-generated identifier reused only for an uncertain acquisition of the same still-valid instance."
          }
        },
        "additionalProperties": false
      },
      "ActivityAction": {
        "type": "string",
        "description": "`account.register`: Records completed account registration with an account target carrying userId and null previousEmail.\n`session.login`: Records successful sign-in with a session target carrying the newly created sessionId.\n`session.logout`: Records completed sign-out with a session target carrying the revoked sessionId.\n`session.revoke`: Records explicit revocation of one session with a session target carrying its sessionId.\n`session.revokeOthers`: Records revocation of the user's other sessions with an account target carrying userId and null previousEmail.\n`session.revokeAll`: Records revocation of all the user's sessions with an account target carrying userId and null previousEmail.\n`session.delegate`: A signed-in user authorized a delegation client; the target is the authorizing session.\n`account.password`: Records a password change authenticated by the current password with an account target carrying userId and null previousEmail.\n`account.passwordReset`: Records password replacement using a recovery credential with an account target carrying userId and null previousEmail.\n`account.resetIssued`: Records issuance of a password recovery credential by a system actor with an account target carrying userId and null previousEmail.\n`account.email`: Records an email change with an account target carrying userId and previousEmail set to the address before the change.\n`network.create`: Records completed network creation with a network target carrying networkId.\n`network.rename`: Records a network name change with a network target carrying networkId.\n`configuration.apply`: Records a committed configuration apply with a configuration target carrying the saved revision, independently of Node application results.\n`membership.role`: Records a member's role change with a membership target carrying membershipId.\n`membership.join`: Records a completed network join with a membership target carrying the new membershipId.\n`membership.remove`: Records a completed membership removal with a membership target carrying membershipId.\n`joinLink.create`: Records creation of a join link with a joinLink target carrying linkId.\n`joinLink.revoke`: Records revocation of a join link with a joinLink target carrying linkId.",
        "enum": [
          "account.register",
          "session.login",
          "session.logout",
          "session.revoke",
          "session.revokeOthers",
          "session.revokeAll",
          "session.delegate",
          "account.password",
          "account.passwordReset",
          "account.resetIssued",
          "account.email",
          "network.create",
          "network.rename",
          "configuration.apply",
          "membership.role",
          "membership.join",
          "membership.remove",
          "joinLink.create",
          "joinLink.revoke"
        ]
      },
      "ActivityActor": {
        "oneOf": [
          {
            "type": "object",
            "description": "User identity with an email snapshot from the time of the activity.",
            "required": [
              "userId",
              "email",
              "type"
            ],
            "properties": {
              "email": {
                "type": [
                  "string",
                  "null"
                ],
                "description": "Email at the time of the event; null means no historical snapshot exists."
              },
              "type": {
                "type": "string",
                "enum": [
                  "user"
                ]
              },
              "userId": {
                "$ref": "#/components/schemas/UserId",
                "description": "Stable identifier of the user who performed the action."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Server-originated action without a user actor.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "system"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Verified user or system responsible for a committed activity."
      },
      "ActivityEvent": {
        "type": "object",
        "description": "Immutable record of a completed management action, excluding secrets and failed requests.",
        "required": [
          "id",
          "at",
          "actor",
          "action",
          "target",
          "beforeRevision",
          "afterRevision",
          "requestId"
        ],
        "properties": {
          "action": {
            "$ref": "#/components/schemas/ActivityAction",
            "description": "Completed management operation represented by this record."
          },
          "actor": {
            "$ref": "#/components/schemas/ActivityActor",
            "description": "Verified user or system that performed the action."
          },
          "afterRevision": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Revision",
                "description": "Configuration revision after the action, or null when no configuration change was involved."
              },
              {
                "type": "null"
              }
            ]
          },
          "at": {
            "type": "string",
            "format": "date-time",
            "description": "Server time at which the action completed."
          },
          "beforeRevision": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Revision",
                "description": "Configuration revision before the action, or null when no configuration change was involved."
              },
              {
                "type": "null"
              }
            ]
          },
          "id": {
            "$ref": "#/components/schemas/ActivityId",
            "description": "Opaque identifier of this activity record."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation ID for the logical operation, not an authorization credential."
          },
          "target": {
            "$ref": "#/components/schemas/ActivityTarget",
            "description": "Stable typed reference to the affected object."
          }
        },
        "additionalProperties": false
      },
      "ActivityId": {
        "type": "string",
        "format": "uuid",
        "description": "Activity identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "ActivityListResponse": {
        "type": "object",
        "description": "Bounded activity page with a fixed initial commit-sequence upper bound.",
        "required": [
          "requestId",
          "items",
          "nextCursor",
          "asOf"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the initial pagination window was established."
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ActivityEvent"
            },
            "description": "Authorized activity records in descending commit order."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Opaque identity- and scope-bound continuation cursor, or null at the end."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "ActivityResponse": {
        "type": "object",
        "description": "One activity record returned under the caller's current network permissions.",
        "required": [
          "requestId",
          "activity"
        ],
        "properties": {
          "activity": {
            "$ref": "#/components/schemas/ActivityEvent",
            "description": "Committed activity belonging to the requested network."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "ActivityTarget": {
        "oneOf": [
          {
            "type": "object",
            "description": "Account affected by registration, credential, or email operations.",
            "required": [
              "userId",
              "previousEmail",
              "type"
            ],
            "properties": {
              "previousEmail": {
                "type": [
                  "string",
                  "null"
                ],
                "description": "For `account.email`: the address before the change. Null otherwise."
              },
              "type": {
                "type": "string",
                "enum": [
                  "account"
                ]
              },
              "userId": {
                "$ref": "#/components/schemas/UserId",
                "description": "Stable identifier of the affected account."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "User session affected by the operation.",
            "required": [
              "sessionId",
              "type"
            ],
            "properties": {
              "sessionId": {
                "$ref": "#/components/schemas/SessionId",
                "description": "Stable identifier of the affected session."
              },
              "type": {
                "type": "string",
                "enum": [
                  "session"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Network affected by creation or rename.",
            "required": [
              "networkId",
              "type"
            ],
            "properties": {
              "networkId": {
                "$ref": "#/components/schemas/NetworkId",
                "description": "Stable identifier of the affected network."
              },
              "type": {
                "type": "string",
                "enum": [
                  "network"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Membership affected by joining, role change, or removal.",
            "required": [
              "membershipId",
              "type"
            ],
            "properties": {
              "membershipId": {
                "$ref": "#/components/schemas/MembershipId",
                "description": "Stable identifier of the affected membership."
              },
              "type": {
                "type": "string",
                "enum": [
                  "membership"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Join link affected by creation or revocation.",
            "required": [
              "linkId",
              "type"
            ],
            "properties": {
              "linkId": {
                "$ref": "#/components/schemas/JoinLinkId",
                "description": "Stable identifier of the affected join link."
              },
              "type": {
                "type": "string",
                "enum": [
                  "joinLink"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Saved network configuration affected by the operation.",
            "required": [
              "revision",
              "type"
            ],
            "properties": {
              "revision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Configuration revision produced by the operation."
              },
              "type": {
                "type": "string",
                "enum": [
                  "configuration"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Stable typed reference to the object affected by a committed activity."
      },
      "AddressClass": {
        "type": "string",
        "description": "Which range an address that is not a node's falls into.",
        "enum": [
          "mesh",
          "private",
          "public"
        ]
      },
      "AddressPools": {
        "type": "object",
        "description": "Canonical, nonoverlapping address pools available for Mesh address allocation.",
        "required": [
          "ipv4",
          "ipv6"
        ],
        "properties": {
          "ipv4": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Canonical IPv4 CIDRs; an empty list disables IPv4 allocation."
          },
          "ipv6": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Canonical IPv6 CIDRs; an empty list disables IPv6 allocation."
          }
        },
        "additionalProperties": false
      },
      "AddressPoolsInput": {
        "type": "object",
        "description": "Address pool input that distinguishes retaining a family from clearing its pools.",
        "properties": {
          "ipv4": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "IPv4 CIDRs; omission retains existing pools or requests initial allocation, while `[]` clears them."
          },
          "ipv6": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "IPv6 CIDRs; omission retains existing pools or requests initial allocation, while `[]` clears them."
          }
        },
        "additionalProperties": false
      },
      "AggregatedLink": {
        "type": "object",
        "description": "Visible endpoint evidence for one channel, without asserting bidirectional business access.",
        "required": [
          "a",
          "b",
          "linkId",
          "transport",
          "initiatorNodeId",
          "evidence",
          "phase",
          "reports"
        ],
        "properties": {
          "a": {
            "$ref": "#/components/schemas/NodeId",
            "description": "First Node in the channel's endpoint pair."
          },
          "b": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Second Node in the channel's endpoint pair."
          },
          "evidence": {
            "$ref": "#/components/schemas/LinkEvidence",
            "description": "Server evaluation of agreement and freshness across endpoint reports."
          },
          "inUse": {
            "type": [
              "boolean",
              "null"
            ],
            "description": "Whether current evidence reports business dependencies, or null when evidence is insufficient."
          },
          "initiatorNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that initiated channel establishment."
          },
          "linkId": {
            "type": "string",
            "description": "Actual channel identifier shared by matching reports."
          },
          "phase": {
            "$ref": "#/components/schemas/LinkPhase",
            "description": "Channel lifecycle phase retained from the reported evidence."
          },
          "reports": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ReportRef"
            },
            "description": "Original Node snapshots contributing this aggregate."
          },
          "since": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Known UTC start of the channel state, or null when evidence is insufficient."
          },
          "transport": {
            "type": "string",
            "description": "Reported transport used by the channel."
          }
        },
        "additionalProperties": false
      },
      "AggregatedPath": {
        "type": "object",
        "description": "One visible path observation with its original reporter and execution context.",
        "required": [
          "pathId",
          "reporter",
          "state"
        ],
        "properties": {
          "pathId": {
            "type": "string",
            "description": "Runtime path identifier from the original Node report."
          },
          "reporter": {
            "$ref": "#/components/schemas/ReportRef",
            "description": "Exact Node snapshot that supplied the path observation."
          },
          "state": {
            "$ref": "#/components/schemas/PathState",
            "description": "Original ordered path, reporter role, evidence stage and validity interval."
          }
        },
        "additionalProperties": false
      },
      "ApplicationPhase": {
        "type": "string",
        "description": "Last accepted result for an original configuration target, independent of current freshness.",
        "enum": [
          "pending",
          "saved",
          "applied",
          "failed"
        ]
      },
      "ApplyFailure": {
        "type": "string",
        "description": "Node-reported step responsible for a failed configuration attempt.",
        "enum": [
          "validation",
          "storage",
          "listeners",
          "underlay",
          "tun",
          "dns",
          "dataplane"
        ]
      },
      "ApplyPhase": {
        "type": "string",
        "description": "Node-reported stage of receiving and installing an execution target.",
        "enum": [
          "receiving",
          "validating",
          "saved",
          "applying",
          "applied",
          "failed"
        ]
      },
      "ApproveEnrollmentRequest": {
        "type": "object",
        "description": "User approval selecting an eligible network for the device.",
        "required": [
          "networkId"
        ],
        "properties": {
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Eligible network in which to register or reauthorize the device."
          }
        },
        "additionalProperties": false
      },
      "ArtifactId": {
        "type": "string",
        "format": "uuid",
        "description": "Immutable dataset artifact identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "ArtifactSummary": {
        "type": "object",
        "description": "Immutable, validated rule-set artifact selected by a resource binding.",
        "required": [
          "id",
          "digest",
          "createdAt",
          "entries"
        ],
        "properties": {
          "createdAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC time the immutable artifact was created, distinct from later refresh checks."
          },
          "digest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Digest of the complete normalized destination content."
          },
          "entries": {
            "type": "integer",
            "format": "int32",
            "description": "Total normalized CIDR, suffix and exact-name entries.",
            "minimum": 0
          },
          "id": {
            "$ref": "#/components/schemas/ArtifactId",
            "description": "Immutable artifact identity used to bind exact historical content."
          }
        },
        "additionalProperties": false
      },
      "AuthOptionsResponse": {
        "type": "object",
        "description": "Public registration settings and input validation rules.",
        "required": [
          "requestId",
          "registrationEnabled",
          "passwordRules",
          "networkNameRules"
        ],
        "properties": {
          "networkNameRules": {
            "$ref": "#/components/schemas/NetworkNameRules",
            "description": "Rules required when creating or renaming a network."
          },
          "passwordRules": {
            "$ref": "#/components/schemas/PasswordRules",
            "description": "Rules required when setting a password."
          },
          "registrationEnabled": {
            "type": "boolean",
            "description": "Whether this Server currently accepts new account registrations."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "AuthSessionResponse": {
        "type": "object",
        "description": "Authenticated account and current session metadata without its bearer credential.",
        "required": [
          "requestId",
          "user",
          "session"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "session": {
            "$ref": "#/components/schemas/SessionMetadata",
            "description": "Identifier and validity period of the current session."
          },
          "user": {
            "$ref": "#/components/schemas/UserView",
            "description": "Account authenticated by the current session."
          }
        },
        "additionalProperties": false
      },
      "AutoProfile": {
        "type": "object",
        "description": "Ordered region-to-profile selection with an explicit fallback.",
        "required": [
          "rules",
          "fallback"
        ],
        "properties": {
          "fallback": {
            "type": "string",
            "description": "Allowed non-`auto` profile used when no region rule matches or no valid region observation exists."
          },
          "rules": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/CountryProfile"
            },
            "description": "Country mappings evaluated in first-match order."
          }
        },
        "additionalProperties": false
      },
      "BoundNodeKey": {
        "type": "object",
        "description": "Public key metadata bound by the Server to one Node and purpose.",
        "required": [
          "id",
          "generation",
          "publicKey"
        ],
        "properties": {
          "generation": {
            "$ref": "#/components/schemas/Version",
            "description": "Key generation used to reject retired authentication material."
          },
          "id": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Stable identifier of this bound key."
          },
          "publicKey": {
            "$ref": "#/components/schemas/NodePublicKey",
            "description": "Public material for this binding; private key material is never included."
          }
        },
        "additionalProperties": false
      },
      "BusinessProtocol": {
        "type": "string",
        "description": "Inner business protocol whose access scope the Server evaluates.",
        "enum": [
          "tcp",
          "udp",
          "icmpEcho"
        ]
      },
      "BusinessTarget": {
        "oneOf": [
          {
            "type": "object",
            "description": "Address a registered Mesh Node.",
            "required": [
              "node",
              "type"
            ],
            "properties": {
              "node": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Destination Node in the demand's network."
              },
              "type": {
                "type": "string",
                "enum": [
                  "mesh"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Address a numeric destination, classified against the configured Mesh pools.",
            "required": [
              "address",
              "type"
            ],
            "properties": {
              "address": {
                "$ref": "#/components/schemas/ExecutionAddress",
                "description": "Exact destination IP address."
              },
              "type": {
                "type": "string",
                "enum": [
                  "ip"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Address a domain through an IP association that must have a current trusted DNS input.",
            "required": [
              "name",
              "address",
              "type"
            ],
            "properties": {
              "address": {
                "$ref": "#/components/schemas/ExecutionAddress",
                "description": "Destination IP that must belong to that association."
              },
              "name": {
                "type": "string",
                "description": "Domain name checked against the source's trusted DNS association."
              },
              "type": {
                "type": "string",
                "enum": [
                  "domain"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Exact business destination submitted in a demand and bound into its compiled permit."
      },
      "CancelDemandRequest": {
        "type": "object",
        "description": "Node request to cancel an existing demand owned by its current instance.",
        "required": [
          "instance",
          "requestId"
        ],
        "properties": {
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current source instance required to cancel the demand."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Existing demand identifier to cancel."
          }
        },
        "additionalProperties": false
      },
      "CancelUdpTraversalResponse": {
        "oneOf": [
          {
            "type": "object",
            "description": "The unfinished acquisition is cancelled, including an identical cancellation retry.",
            "required": [
              "status"
            ],
            "properties": {
              "status": {
                "type": "string",
                "enum": [
                  "cancelled"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The attempt could not be cancelled under the supplied current bindings.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/UdpTraversalUnavailableReason",
                "description": "Closed coordination condition that prevented cancellation."
              },
              "status": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Cancellation ends unfinished acquisition only, never an established business link."
      },
      "CandidateReference": {
        "type": "object",
        "description": "Immutable reference to an original State-owned endpoint observation without disclosing its private source details.",
        "required": [
          "socket",
          "observationDigest"
        ],
        "properties": {
          "observationDigest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Digest of the original address, source, observation lifetime and socket binding, excluding report sequence and receipt time."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketReference",
            "description": "Exact socket that produced and will use the candidate."
          }
        },
        "additionalProperties": false
      },
      "CandidateSource": {
        "type": "string",
        "description": "Candidate provenance exposed to peers without private interface or observer details.",
        "enum": [
          "configured",
          "interface",
          "stun"
        ]
      },
      "Capability": {
        "oneOf": [
          {
            "type": "object",
            "description": "Supported connection directions and stream encapsulations for a transport.",
            "required": [
              "roles",
              "streams",
              "kind"
            ],
            "properties": {
              "kind": {
                "type": "string",
                "enum": [
                  "transport"
                ]
              },
              "roles": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/TransportRole"
                },
                "description": "Connection directions actually supported by this build."
              },
              "streams": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/StreamKind"
                },
                "description": "Stream encapsulations actually supported by this build."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Support for a versioned feature identified by the capability map key.",
            "required": [
              "version",
              "kind"
            ],
            "properties": {
              "kind": {
                "type": "string",
                "enum": [
                  "feature"
                ]
              },
              "version": {
                "type": "integer",
                "format": "int32",
                "description": "Protocol version implemented for this feature.",
                "minimum": 0
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Implementation capability declared by the Node, without granting execution permission."
      },
      "Carrier": {
        "type": "object",
        "description": "Actual socket establishment orientation, independent of business direction.",
        "required": [
          "dialer",
          "listener",
          "listenerKey",
          "protocol"
        ],
        "properties": {
          "dialer": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that initiates the underlying connection."
          },
          "listener": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that accepts the underlying connection."
          },
          "listenerKey": {
            "type": "string",
            "description": "Configuration key of the accepting Node's listener."
          },
          "protocol": {
            "$ref": "#/components/schemas/TransportProtocol",
            "description": "Protocol encapsulation authorized for this adjacency."
          }
        },
        "additionalProperties": false
      },
      "ChallengeId": {
        "type": "string",
        "format": "uuid",
        "description": "One-use proof challenge identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "ChangeEmailRequest": {
        "type": "object",
        "description": "Login email replacement authorized by the account password.",
        "required": [
          "password",
          "email"
        ],
        "properties": {
          "email": {
            "type": "string",
            "description": "New login address, normalized using the registration email rules."
          },
          "password": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Current account password used to authorize the change."
          }
        },
        "additionalProperties": false
      },
      "ChangeKind": {
        "type": "string",
        "description": "Structural difference at a configuration JSON Pointer.",
        "enum": [
          "added",
          "removed",
          "changed"
        ]
      },
      "ChangeMemberRoleRequest": {
        "type": "object",
        "description": "Membership role change guarded by the previously read member version.",
        "required": [
          "expectedVersion",
          "role"
        ],
        "properties": {
          "expectedVersion": {
            "$ref": "#/components/schemas/Version",
            "description": "Member version that must still match when the role change is committed."
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole",
            "description": "Requested role, subject to the caller's assignable roles and last-Owner protection."
          }
        },
        "additionalProperties": false
      },
      "ChangePasswordRequest": {
        "type": "object",
        "description": "Password replacement authorized by the account's current password.",
        "required": [
          "currentPassword",
          "password"
        ],
        "properties": {
          "currentPassword": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Current account password, without trimming or rewriting."
          },
          "password": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Replacement password satisfying the advertised password rules."
          }
        },
        "additionalProperties": false
      },
      "ClientTls": {
        "type": "object",
        "description": "Target-bound TLS verification material and client options resolved from the selected listener and dialer.",
        "required": [
          "serverName",
          "certificateChain",
          "options"
        ],
        "properties": {
          "certificateChain": {
            "$ref": "#/components/schemas/SecretString",
            "description": "Target listener's PEM certificate chain, leaf first, used for trust roots and exact leaf-certificate verification."
          },
          "options": {
            "$ref": "#/components/schemas/DialerTls",
            "description": "Selected dialer's TLS version, ALPN, trust, and handshake-budget options."
          },
          "serverName": {
            "type": "string",
            "description": "TLS verification name supplied by the target listener, distinct from its endpoint host and WebSocket Host."
          }
        },
        "additionalProperties": false
      },
      "CloseReason": {
        "type": "string",
        "description": "Node-reported reason a channel or connection attempt ended.",
        "enum": [
          "idle",
          "remote_closed",
          "timeout",
          "withdrawn",
          "local_failure"
        ]
      },
      "ClosedReason": {
        "type": "string",
        "description": "Explicit reason for ending an event stream before EOF.",
        "enum": [
          "session_expired",
          "membership_revoked",
          "node_lease_expired",
          "server_shutdown"
        ]
      },
      "CodeChallengeMethod": {
        "type": "string",
        "description": "PKCE transform of the verifier; only S256 is accepted.",
        "enum": [
          "S256"
        ]
      },
      "CompareConfigurationRequest": {
        "type": "object",
        "description": "Two saved source revisions in the requested network to compare.",
        "required": [
          "fromRevision",
          "toRevision"
        ],
        "properties": {
          "fromRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Baseline revision for the comparison."
          },
          "toRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Target revision compared with the baseline."
          }
        },
        "additionalProperties": false
      },
      "CompiledNode": {
        "type": "object",
        "description": "Complete Server-compiled Node target paired with the digest checked before persistence and installation.",
        "required": [
          "digest",
          "configuration"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/NodeConfiguration",
            "description": "Immutable execution content whose digest is supplied."
          },
          "digest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Digest of the complete configuration value."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationAppliedResponse": {
        "type": "object",
        "description": "Committed configuration revision and its original execution targets, without asserting node installation.",
        "required": [
          "requestId",
          "networkId",
          "revision",
          "publicationIds",
          "targetCount",
          "deletedNodeIds"
        ],
        "properties": {
          "deletedNodeIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Nodes permanently removed and revoked by this commit."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network whose configuration was committed."
          },
          "publicationIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PublicationId"
            },
            "description": "Execution publications containing this apply's node targets; empty when there are no targets."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for the configuration apply request."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Source revision resulting from the accepted apply."
          },
          "targetCount": {
            "type": "integer",
            "format": "int32",
            "description": "Original node targets counted once per node, independent of later demand or refresh publications.",
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "ConfigurationApplyProgress": {
        "type": "object",
        "description": "Application progress for a source revision's fixed original target set, filtered by current caller permissions.",
        "required": [
          "networkId",
          "revision",
          "scope",
          "summary",
          "nodes"
        ],
        "properties": {
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network whose apply progress is reported."
          },
          "nodes": {
            "type": "object",
            "description": "Original visible node targets and their last results, unaffected by later targets added by demands or refreshes.",
            "additionalProperties": {
              "$ref": "#/components/schemas/ConfigurationNodeProgress"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Source revision that originally established the reported targets."
          },
          "scope": {
            "$ref": "#/components/schemas/ProgressScope",
            "description": "Whether counts and node details cover all original targets or only the visible subset."
          },
          "summary": {
            "$ref": "#/components/schemas/ConfigurationProgressSummary",
            "description": "Phase totals over exactly the nodes included in this projection."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationChange": {
        "type": "object",
        "description": "Secret-free summary of one final configuration or stable-binding difference.",
        "required": [
          "path",
          "kind"
        ],
        "properties": {
          "kind": {
            "$ref": "#/components/schemas/ChangeKind",
            "description": "Whether the target adds, removes or changes the value at this pointer."
          },
          "path": {
            "type": "string",
            "description": "JSON Pointer to the deepest differing value, or the added or removed member as a whole."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationChangeCause": {
        "type": "string",
        "description": "Primary trigger for a saved configuration revision, independent of Node application status.",
        "enum": [
          "networkCreated",
          "edit",
          "restore",
          "enrollment",
          "nodeDeleted",
          "emailSync",
          "memberRemoved",
          "identityTransition"
        ]
      },
      "ConfigurationComparison": {
        "type": "object",
        "description": "Secret-free comparison of source text, canonical values and stable bindings between saved revisions.",
        "required": [
          "requestId",
          "fromRevision",
          "toRevision",
          "sourceChanged",
          "configurationChanged",
          "bindingsChanged",
          "changes"
        ],
        "properties": {
          "bindingsChanged": {
            "type": "boolean",
            "description": "Whether stable user, node identity, listener or rule-set bindings differ."
          },
          "changes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConfigurationChange"
            },
            "description": "JSON Pointer differences for configuration values and stable bindings, without secret before/after values."
          },
          "configurationChanged": {
            "type": "boolean",
            "description": "Whether canonical configuration values differ."
          },
          "fromRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Baseline source revision."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for this HTTP request."
          },
          "sourceChanged": {
            "type": "boolean",
            "description": "Whether original JSONC text differs, including comments and formatting."
          },
          "toRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Target source revision."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationIssue": {
        "type": "object",
        "description": "Structured configuration error, warning or retained unavailable dependency, without sensitive input values in its message.",
        "required": [
          "path",
          "code",
          "params",
          "message"
        ],
        "properties": {
          "code": {
            "$ref": "#/components/schemas/IssueCode",
            "description": "Stable condition identifier that determines the parameter keys and message rendering."
          },
          "message": {
            "type": "string",
            "description": "English rendering of `code`; not for programmatic use."
          },
          "params": {
            "type": "object",
            "description": "Parameters named by the code's documentation. Always present.",
            "additionalProperties": {
              "type": "string"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "path": {
            "type": "string",
            "description": "JSON pointer into the configuration input; empty for the document."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationNodeProgress": {
        "type": "object",
        "description": "Historical application result and independent freshness for one original node target.",
        "required": [
          "target",
          "phase",
          "freshness"
        ],
        "properties": {
          "attempt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Version",
                "description": "Installation attempt identified by the last accepted receipt; null when no receipt exists."
              },
              {
                "type": "null"
              }
            ]
          },
          "freshness": {
            "$ref": "#/components/schemas/ReceiptFreshness",
            "description": "Independent of historical result; never implies data-plane health."
          },
          "instanceId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/InstanceId",
                "description": "Instance that submitted the last accepted result; null when no receipt exists."
              },
              {
                "type": "null"
              }
            ]
          },
          "phase": {
            "$ref": "#/components/schemas/ApplicationPhase",
            "description": "Last result for the original target, including historical instances."
          },
          "receivedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "UTC time the Server received that result, rather than its observation time; null when no receipt exists."
          },
          "sequence": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Sequence",
                "description": "Receipt sequence within that attempt; null when no receipt exists."
              },
              {
                "type": "null"
              }
            ]
          },
          "target": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact execution reference fixed for this node by the original configuration apply."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationOperation": {
        "oneOf": [
          {
            "type": "object",
            "description": "Replace all business access grants.",
            "required": [
              "access",
              "type"
            ],
            "properties": {
              "access": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/AccessRule"
                },
                "description": "Complete replacement grant list; an empty list denies new Mesh business traffic."
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceAccess"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Replace all adjacency rules.",
            "required": [
              "topology",
              "type"
            ],
            "properties": {
              "topology": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/TopologyRule"
                },
                "description": "Complete replacement topology list; an empty list uses the network topology default."
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceTopology"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Replace Mesh and egress routing declarations together.",
            "required": [
              "routing",
              "type"
            ],
            "properties": {
              "routing": {
                "$ref": "#/components/schemas/RoutingConfiguration",
                "description": "Complete replacement routing settings, including profiles and assignments."
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceRouting"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Replace network DNS settings.",
            "required": [
              "dns",
              "type"
            ],
            "properties": {
              "dns": {
                "$ref": "#/components/schemas/DnsConfiguration",
                "description": "Complete replacement naming, resolver and split-DNS configuration."
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceDns"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Update explicitly provided network default fields.",
            "required": [
              "defaults",
              "type"
            ],
            "properties": {
              "defaults": {
                "$ref": "#/components/schemas/DefaultsUpdate",
                "description": "Default-field updates; omitted fields retain their baseline values."
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceDefaults"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Replace all user group definitions.",
            "required": [
              "groups",
              "type"
            ],
            "properties": {
              "groups": {
                "type": "object",
                "description": "`group:<name>` keys mapped to direct member emails; an empty map removes all groups.",
                "additionalProperties": {
                  "type": "array",
                  "items": {
                    "type": "string"
                  }
                },
                "propertyNames": {
                  "type": "string"
                }
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceGroups"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Replace all tag definitions and assignment owners.",
            "required": [
              "tagsOwners",
              "type"
            ],
            "properties": {
              "tagsOwners": {
                "type": "object",
                "description": "`tag:<name>` keys mapped to owner emails or groups; an empty owner list allows only network administrators to assign the tag.",
                "additionalProperties": {
                  "type": "array",
                  "items": {
                    "type": "string"
                  }
                },
                "propertyNames": {
                  "type": "string"
                }
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceTagsOwners"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Replace all destination rule-set declarations.",
            "required": [
              "ruleSets",
              "type"
            ],
            "properties": {
              "ruleSets": {
                "type": "object",
                "description": "Complete named rule-set map; remote contents are prepared and validated before publication.",
                "additionalProperties": {
                  "$ref": "#/components/schemas/RuleSet"
                },
                "propertyNames": {
                  "type": "string"
                }
              },
              "type": {
                "type": "string",
                "enum": [
                  "replaceRuleSets"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Update allowed settings of an existing node.",
            "required": [
              "nodeId",
              "fields",
              "type"
            ],
            "properties": {
              "fields": {
                "$ref": "#/components/schemas/NodeSettingsPatch",
                "description": "Explicit field changes, preserving every omitted setting."
              },
              "nodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Existing node in the target network whose settings are edited."
              },
              "type": {
                "type": "string",
                "enum": [
                  "setNodeSettings"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Create, replace or remove one fixed listener.",
            "required": [
              "nodeId",
              "key",
              "listener",
              "type"
            ],
            "properties": {
              "key": {
                "type": "string",
                "description": "Nonempty node-local listener key."
              },
              "listener": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/ListenerUpdate",
                    "description": "Replacement listener with secret-retention semantics, or explicit null to remove it; this field is required."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "nodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Existing node that owns the listener."
              },
              "type": {
                "type": "string",
                "enum": [
                  "setListener"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Set a node's tag authorization identity subject to assignment and identity-transition permissions.",
            "required": [
              "nodeId",
              "tags",
              "type"
            ],
            "properties": {
              "nodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Existing node whose effective identity is changed."
              },
              "tags": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Nonempty, duplicate-free `tag:<name>` references; clearing the last tag cannot restore a user identity."
              },
              "type": {
                "type": "string",
                "enum": [
                  "setIdentityTags"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Update the network's allocatable Mesh pools.",
            "required": [
              "addressPools",
              "type"
            ],
            "properties": {
              "addressPools": {
                "$ref": "#/components/schemas/AddressPoolsInput",
                "description": "Per-family pool replacements; omission retains existing pools and `[]` removes a family's pools."
              },
              "type": {
                "type": "string",
                "enum": [
                  "setAddressPools"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Update one node's desired Mesh addresses.",
            "required": [
              "nodeId",
              "addresses",
              "type"
            ],
            "properties": {
              "addresses": {
                "$ref": "#/components/schemas/NodeAddressesInput",
                "description": "Per-family address updates; omission retains an assignment and null explicitly clears it."
              },
              "nodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Existing node whose address assignments are edited."
              },
              "type": {
                "type": "string",
                "enum": [
                  "setAddresses"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Permanently delete a node, revoke its credentials and retire its addresses on commit.",
            "required": [
              "nodeId",
              "type"
            ],
            "properties": {
              "nodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Existing node to remove permanently; historical restore cannot revive it."
              },
              "type": {
                "type": "string",
                "enum": [
                  "deleteNode"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Typed edit applied in request order, with per-operation authorization and atomic full-target validation under CAS."
      },
      "ConfigurationPreviewResponse": {
        "type": "object",
        "description": "Redacted, non-executable draft comparison that reserves no revision, address or permanent material.",
        "required": [
          "preparedRuleSets",
          "unavailableDependencies",
          "warnings",
          "requestId",
          "baseRevision",
          "preview",
          "changeCount",
          "changes",
          "sourceChanged",
          "deletedNodeIds",
          "addressChangedNodeIds",
          "identityChangedNodeIds",
          "changedListeners",
          "affectedNodeIds"
        ],
        "properties": {
          "addressChangedNodeIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Nodes with desired Mesh address changes that retire old bindings and interrupt affected sessions."
          },
          "affectedNodeIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Nodes whose execution target may change: every node unless all changes\nare confined to node-local display fields (`hostname`, `remark`) and\nthe bindings of unchanged values."
          },
          "baseRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Actual current revision used as the comparison baseline."
          },
          "changeCount": {
            "type": "integer",
            "format": "int32",
            "description": "Number of final effective configuration and binding differences, equal to the length of `changes`.",
            "minimum": 0
          },
          "changedListeners": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ListenerChange"
            },
            "description": "Listeners added, removed or changed, including material to be generated."
          },
          "changes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConfigurationChange"
            },
            "description": "Complete secret-free summary of final differences, rather than an operation or edit count."
          },
          "deletedNodeIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Nodes that apply would permanently delete and revoke."
          },
          "identityChangedNodeIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Nodes whose identity or its stable binding changes."
          },
          "preparedRuleSets": {
            "type": "object",
            "description": "Transient canonical content checked by validate; no artifact/version reservation.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PreparedRuleSet"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "preview": {
            "$ref": "#/components/schemas/ConfigurationView",
            "description": "Redacted normalized target declaration, with pending automatic materials still absent."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for this validation request."
          },
          "requirements": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PreparationRequirement"
            },
            "description": "Material the author still has to complete or that awaits verification.\nDisjoint from `willGenerate`; preview is never executable."
          },
          "sourceChanged": {
            "type": "boolean",
            "description": "Whether the JSONC source text changes, including comments without effective value changes."
          },
          "unavailableDependencies": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConfigurationIssue"
            },
            "description": "Retained references whose dependencies are revoked or inactive, without restoring their execution authority."
          },
          "warnings": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConfigurationIssue"
            },
            "description": "Accepted as written but probably unintended, e.g. a shadowed rule."
          },
          "willGenerate": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PreparationRequirement"
            },
            "description": "Values that apply will generate, after rechecking authorization and CAS."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationProgress": {
        "type": "object",
        "description": "Node-reported progress for one exact execution target.",
        "required": [
          "target",
          "phase",
          "observedAt"
        ],
        "properties": {
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC time at which the Node observed this stage."
          },
          "phase": {
            "$ref": "#/components/schemas/ApplyPhase",
            "description": "Actual processing or installation stage observed by the Node."
          },
          "problem": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/StateProblem",
                "description": "Reported failure details, or null when no problem is reported."
              },
              {
                "type": "null"
              }
            ]
          },
          "target": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact configuration and execution publication being processed."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationProgressSummary": {
        "type": "object",
        "description": "Counts of original visible targets by last accepted phase, without interpreting historical results as health.",
        "required": [
          "targetCount",
          "pending",
          "saved",
          "applied",
          "failed"
        ],
        "properties": {
          "applied": {
            "type": "integer",
            "format": "int32",
            "description": "Original visible targets whose last accepted result is applied, even if subsequently expired or superseded.",
            "minimum": 0
          },
          "failed": {
            "type": "integer",
            "format": "int32",
            "description": "Original visible targets whose last accepted result is failed.",
            "minimum": 0
          },
          "pending": {
            "type": "integer",
            "format": "int32",
            "description": "Original visible targets with no accepted result.",
            "minimum": 0
          },
          "saved": {
            "type": "integer",
            "format": "int32",
            "description": "Original visible targets whose last accepted result is saved.",
            "minimum": 0
          },
          "targetCount": {
            "type": "integer",
            "format": "int32",
            "description": "Counts of the original targets' last accepted phase, not online/healthy\nnodes. Inspect each node's independent freshness before presenting it as\ncurrently applied; expired/superseded results remain historical evidence.",
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "ConfigurationRef": {
        "type": "object",
        "description": "Exact Node execution target binding source revision, publication and compiled content.",
        "required": [
          "revision",
          "executionPublication",
          "digest"
        ],
        "properties": {
          "digest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Digest of this Node's complete compiled target, which may differ from other Nodes' digests."
          },
          "executionPublication": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Publication fixing the target's identity, material and execution dependencies."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Source configuration revision used to compile the target."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationRequest": {
        "oneOf": [
          {
            "type": "object",
            "description": "Replace the configuration from a complete JSONC document.",
            "required": [
              "expectedRevision",
              "requestId",
              "sourceJsonc",
              "type"
            ],
            "properties": {
              "expectedRevision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Current network revision required by strict compare-and-swap validation."
              },
              "requestId": {
                "$ref": "#/components/schemas/RequestId",
                "description": "Apply idempotency key; reuse with a different request body returns `request_id_conflict`."
              },
              "sourceJsonc": {
                "$ref": "#/components/schemas/WireSecret",
                "description": "Full JSONC source including comments and any secret material; handled as sensitive input."
              },
              "type": {
                "type": "string",
                "enum": [
                  "document"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Apply ordered typed edits to the current JSONC baseline as one atomic change.",
            "required": [
              "expectedRevision",
              "requestId",
              "ops",
              "type"
            ],
            "properties": {
              "expectedRevision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Current network revision required by strict compare-and-swap validation."
              },
              "ops": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/ConfigurationOperation"
                },
                "description": "Ordered edits, each separately authorized before validating and committing the complete target."
              },
              "requestId": {
                "$ref": "#/components/schemas/RequestId",
                "description": "Apply idempotency key; reuse with a different request body returns `request_id_conflict`."
              },
              "type": {
                "type": "string",
                "enum": [
                  "operations"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Restore a historical configuration as a new revision under current identity and dependency checks.",
            "required": [
              "expectedRevision",
              "requestId",
              "restoreFrom",
              "type"
            ],
            "properties": {
              "expectedRevision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Current network revision required by strict compare-and-swap validation."
              },
              "requestId": {
                "$ref": "#/components/schemas/RequestId",
                "description": "Apply idempotency key; reuse with a different request body returns `request_id_conflict`."
              },
              "resourcePublication": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/PublicationId",
                    "description": "Omitted: the source version's initial resource bindings. Otherwise\nselect this exact historical publication from the same source version."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "restoreFrom": {
                "$ref": "#/components/schemas/Revision",
                "description": "Historical source revision in the same network whose configuration is restored."
              },
              "type": {
                "type": "string",
                "enum": [
                  "restore"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Complete configuration edit or historical restore, checked against the current revision before commit."
      },
      "ConfigurationResponse": {
        "type": "object",
        "description": "Redacted configuration snapshot at an exact network source revision.",
        "required": [
          "requestId",
          "networkId",
          "revision",
          "configuration"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationView",
            "description": "Desired configuration with credentials and sensitive header values removed."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network that owns the configuration."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for this HTTP request."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Exact source revision represented by this snapshot."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationRevisionList": {
        "type": "object",
        "description": "Configuration history page bounded by the maximum revision of the initial query.",
        "required": [
          "requestId",
          "currentRevision",
          "items",
          "nextCursor",
          "asOf"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the initial pagination window was established."
          },
          "currentRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Current desired configuration revision at this page read."
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConfigurationRevisionSummary"
            },
            "description": "Version summaries in descending revision order."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Opaque identity- and network-bound continuation cursor, or null at the end."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationRevisionSummary": {
        "type": "object",
        "description": "Non-secret summary of one persistently saved configuration version.",
        "required": [
          "revision",
          "createdAt",
          "createdBy",
          "cause",
          "current",
          "rollbackFrom"
        ],
        "properties": {
          "cause": {
            "$ref": "#/components/schemas/ConfigurationChangeCause",
            "description": "Primary trigger that produced this configuration revision."
          },
          "createdAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which this revision was persistently committed."
          },
          "createdBy": {
            "$ref": "#/components/schemas/ActivityActor",
            "description": "Verified committing actor with its historical email snapshot when available."
          },
          "current": {
            "type": "boolean",
            "description": "Whether this revision equals the current desired revision at this read."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Historical configuration revision represented by this entry."
          },
          "rollbackFrom": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Revision",
                "description": "Source revision used for a restore, or null for a non-restore change."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "ConfigurationSchemaResponse": {
        "type": "object",
        "description": "Complete configuration schema document for validating and editing network declarations.",
        "required": [
          "requestId",
          "schemaJson"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for this HTTP request."
          },
          "schemaJson": {
            "type": "string",
            "description": "A complete JSON Schema 2020-12 document encoded as text for editor use.\nIt contains definitions and constraints, never configuration values."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationSourceResponse": {
        "type": "object",
        "description": "Sensitive source and canonical configuration for authorized full-document editing.",
        "required": [
          "requestId",
          "networkId",
          "revision",
          "sourceJsonc",
          "configuration"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/NetworkConfiguration",
            "description": "Complete canonical configuration including fixed defaults and prepared secret material."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network that owns the configuration."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for this HTTP request."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Exact source revision represented by both document forms."
          },
          "sourceJsonc": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Original JSONC text including comments and secret values."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationState": {
        "type": "object",
        "description": "Node-reported persistence, execution and progress facts, which do not prove business connectivity.",
        "properties": {
          "applied": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Configuration used by the current running generation, or null if none is applied."
              },
              {
                "type": "null"
              }
            ]
          },
          "progress": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationProgress",
                "description": "Current target progress, or null when no progress is reported."
              },
              {
                "type": "null"
              }
            ]
          },
          "saved": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Complete validated configuration privately persisted by the Node, or null if none is reported."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "ConfigurationValidationParams": {
        "type": "object",
        "description": "Typed configuration errors and preparation requirements reported by validation.",
        "required": [
          "issues",
          "requirements"
        ],
        "properties": {
          "issues": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ConfigurationIssue"
            },
            "description": "Validation issues located by JSON Pointer in the submitted input."
          },
          "requirements": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PreparationRequirement"
            },
            "description": "Material, address, or other preparation requirements for the submitted configuration."
          }
        },
        "additionalProperties": false
      },
      "ConfigurationView": {
        "type": "object",
        "description": "Redacted network configuration projection used for reads and non-executable previews.",
        "required": [
          "addressPools",
          "defaults",
          "groups",
          "tagsOwners",
          "access",
          "nodes",
          "dns",
          "topology",
          "routing",
          "ruleSets"
        ],
        "properties": {
          "access": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/AccessRule"
            },
            "description": "Additive business traffic grants, with no grant meaning denial."
          },
          "addressPools": {
            "$ref": "#/components/schemas/AddressPools",
            "description": "Network-wide usable Mesh address pools."
          },
          "defaults": {
            "$ref": "#/components/schemas/DefaultsView",
            "description": "Unmatched-rule behavior and redacted inherited dialers."
          },
          "dns": {
            "$ref": "#/components/schemas/DnsConfiguration",
            "description": "Network naming and explicit resolver routing."
          },
          "groups": {
            "type": "object",
            "description": "`group:<name>` definitions containing direct network member emails.",
            "additionalProperties": {
              "type": "array",
              "items": {
                "type": "string"
              }
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "nodes": {
            "type": "object",
            "description": "Registered node IDs mapped to redacted desired settings.",
            "additionalProperties": {
              "$ref": "#/components/schemas/NodeSettingsView"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "routing": {
            "$ref": "#/components/schemas/RoutingConfiguration",
            "description": "Mesh waypoint requirements and external destination routing."
          },
          "ruleSets": {
            "type": "object",
            "description": "Named destination-set declarations; downloaded content and check status are exposed by resource views.",
            "additionalProperties": {
              "$ref": "#/components/schemas/RuleSet"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "tagsOwners": {
            "type": "object",
            "description": "`tag:<name>` definitions containing owner emails or `group:<name>` references.",
            "additionalProperties": {
              "type": "array",
              "items": {
                "type": "string"
              }
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "topology": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TopologyRule"
            },
            "description": "Undirected adjacency rules, evaluated independently of business access grants."
          }
        },
        "additionalProperties": false
      },
      "Count": {
        "type": "integer",
        "description": "A nonnegative item count bounded for lossless JSON arithmetic.",
        "maximum": 9007199254740991,
        "minimum": 0
      },
      "CountryProfile": {
        "type": "object",
        "description": "Region condition used by the automatic profile selector.",
        "required": [
          "countries",
          "profile"
        ],
        "properties": {
          "countries": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Nonempty list of uppercase two-letter country codes matched against the observed public underlay address's region."
          },
          "profile": {
            "type": "string",
            "description": "Profile selected on a match; it must be allowed and cannot recursively select `auto`."
          }
        },
        "additionalProperties": false
      },
      "CoverageReason": {
        "type": "string",
        "description": "Server policy responsible for pruning historical coverage.",
        "enum": [
          "retention",
          "capacity"
        ]
      },
      "CreateDelegationRequest": {
        "type": "object",
        "description": "A signed-in user's request to let a configured browser client act on their behalf.",
        "required": [
          "client",
          "redirectUri",
          "codeChallenge",
          "codeChallengeMethod"
        ],
        "properties": {
          "client": {
            "type": "string",
            "description": "Name of a delegation client configured on the Server."
          },
          "codeChallenge": {
            "type": "string",
            "description": "base64url without padding of the SHA-256 of the client's verifier (43 characters)."
          },
          "codeChallengeMethod": {
            "$ref": "#/components/schemas/CodeChallengeMethod"
          },
          "redirectUri": {
            "type": "string",
            "description": "Absolute `http(s)` URL on the client's origin that will receive the code; no fragment or credentials."
          }
        },
        "additionalProperties": false
      },
      "CreateEnrollmentKeyRequest": {
        "type": "object",
        "description": "Administrator request for a one-time, expiring Tag enrollment key.",
        "required": [
          "requestId",
          "tags",
          "initialSettings",
          "expiresAt"
        ],
        "properties": {
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC deadline after which the key cannot enroll a Node."
          },
          "initialSettings": {
            "$ref": "#/components/schemas/InitialNodeSettings",
            "description": "Initial Node settings validated before the key is created."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Idempotency identifier bound to the caller, network and exact creation request."
          },
          "tags": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Preassigned Tags that define the enrolled Node's authorization identity."
          }
        },
        "additionalProperties": false
      },
      "CreateEnrollmentRequest": {
        "type": "object",
        "description": "Device request to begin enrollment with proof of control-key possession.",
        "required": [
          "intent",
          "signature"
        ],
        "properties": {
          "intent": {
            "$ref": "#/components/schemas/EnrollmentIntent",
            "description": "Exact enrollment intent covered by the device signature."
          },
          "signature": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Control-key signature over the enrollment transcript, encoded as unpadded base64url."
          }
        },
        "additionalProperties": false
      },
      "CreateJoinLinkRequest": {
        "type": "object",
        "description": "Request to issue a one-time network join link with a fixed role and lifetime.",
        "required": [
          "requestId",
          "role",
          "expiresInSeconds"
        ],
        "properties": {
          "expiresInSeconds": {
            "type": "integer",
            "format": "int32",
            "description": "Link lifetime in seconds, from 60 through 604800.",
            "maximum": 604800,
            "minimum": 60
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Caller-chosen idempotency key bound to the caller, network, and creation input."
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole",
            "description": "Role granted on redemption, restricted to roles the issuer may currently grant."
          }
        },
        "additionalProperties": false
      },
      "CreateNetworkRequest": {
        "type": "object",
        "description": "Request to create an isolated network and an Owner membership for the caller.",
        "required": [
          "requestId",
          "name"
        ],
        "properties": {
          "name": {
            "type": "string",
            "description": "Raw input. Apply getAuthOptions.networkNameRules; padding is allowed."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Caller-chosen idempotency key bound to the authenticated user and creation input."
          }
        },
        "additionalProperties": false
      },
      "CreateNodeObservation": {
        "type": "object",
        "description": "Node request for an independently leased diagnostic subscription sourced from its authenticated runtime instance.",
        "required": [
          "instance",
          "subscriptionId",
          "target",
          "service",
          "selection",
          "metrics",
          "intervalMs"
        ],
        "properties": {
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current source runtime instance to which the subscription is bound."
          },
          "intervalMs": {
            "type": "integer",
            "format": "int32",
            "description": "Requested sampling interval in milliseconds, from 1000 through 30000 and subject to execution budgets.",
            "maximum": 30000,
            "minimum": 1000
          },
          "metrics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MetricKind"
            },
            "description": "Requested measurement kinds; unsupported kinds are reported explicitly by the task."
          },
          "selection": {
            "$ref": "#/components/schemas/ObservationPath",
            "description": "Automatic authorized path selection or a fixed complete ordered path."
          },
          "service": {
            "$ref": "#/components/schemas/ObservationServiceScope",
            "description": "Exact TCP, UDP or ICMP scope checked by Access and complete-path routing."
          },
          "subscriptionId": {
            "$ref": "#/components/schemas/ObservationSubscriptionId",
            "description": "Independent subscription identifier reused with the same intent on creation retries."
          },
          "target": {
            "$ref": "#/components/schemas/NodeObservationTarget",
            "description": "Configured Node name or exact Mesh IP to resolve under current permissions."
          }
        },
        "additionalProperties": false
      },
      "CreateObservation": {
        "type": "object",
        "description": "User request for a short-lived, independently authorized diagnostic subscription.",
        "required": [
          "subscriptionId",
          "sourceNodeId",
          "targetNodeId",
          "selection",
          "service",
          "metrics",
          "intervalMs"
        ],
        "properties": {
          "intervalMs": {
            "type": "integer",
            "format": "int32",
            "description": "Requested sampling interval in milliseconds, from 1000 through 30000 and subject to budgets.",
            "maximum": 30000,
            "minimum": 1000
          },
          "metrics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MetricKind"
            },
            "description": "Requested measurement kinds; unavailable kinds are returned separately as unsupported."
          },
          "selection": {
            "$ref": "#/components/schemas/ObservationPath",
            "description": "Automatic authorized selection or an explicit complete path."
          },
          "service": {
            "$ref": "#/components/schemas/ObservationServiceScope",
            "description": "Exact TCP, UDP or ICMP service scope checked by access and routing policy."
          },
          "sourceNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node authorized to originate the measurement; ordinary members may use only their own User Nodes."
          },
          "subscriptionId": {
            "$ref": "#/components/schemas/ObservationSubscriptionId",
            "description": "Independent subscription identifier reused for bounded idempotent creation retries."
          },
          "targetNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Business destination Node within the current network."
          }
        },
        "additionalProperties": false
      },
      "DecisionDelta": {
        "type": "object",
        "description": "Server-computed keyed changes between two complete execution targets.",
        "required": [
          "remove",
          "upsert"
        ],
        "properties": {
          "remove": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/RequestId"
            },
            "description": "Keys present in the base that must be removed from the reconstructed target."
          },
          "upsert": {
            "type": "object",
            "description": "Complete values to insert or replace; keys cannot also occur in remove.",
            "additionalProperties": {
              "$ref": "#/components/schemas/DemandDecision"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "Defaults": {
        "type": "object",
        "description": "Network-wide unmatched-rule behavior and inherited outbound transport selection.",
        "properties": {
          "access": {
            "$ref": "#/components/schemas/AccessDefault",
            "description": "Unmatched Mesh business traffic is denied; defaults to `deny`, the only supported value."
          },
          "dialers": {
            "$ref": "#/components/schemas/DialerSet",
            "description": "Outbound protocol set inherited by nodes; omission uses the saved platform baseline and `{}` disables active dialing."
          },
          "topology": {
            "$ref": "#/components/schemas/TopologyDefault",
            "description": "Whether unmatched node pairs may form adjacency; defaults to `allow`."
          }
        },
        "additionalProperties": false
      },
      "DefaultsUpdate": {
        "type": "object",
        "description": "Network default edits that preserve every omitted baseline field.",
        "properties": {
          "access": {
            "$ref": "#/components/schemas/AccessDefault",
            "description": "Unmatched Mesh traffic behavior; omission retains the current value and the only explicit value is `deny`."
          },
          "dialers": {
            "$ref": "#/components/schemas/DialerSetUpdate",
            "description": "Replacement default protocol set; omission retains it and `{}` disables network-default active dialing."
          },
          "topology": {
            "$ref": "#/components/schemas/TopologyDefault",
            "description": "Unmatched adjacency permission; omission retains the current value."
          }
        },
        "additionalProperties": false
      },
      "DefaultsView": {
        "type": "object",
        "description": "Public projection of network defaults with sensitive dialer values removed.",
        "required": [
          "access",
          "topology",
          "dialers"
        ],
        "properties": {
          "access": {
            "$ref": "#/components/schemas/AccessDefault",
            "description": "Denial applied to Mesh business traffic without an explicit access grant."
          },
          "dialers": {
            "$ref": "#/components/schemas/DialerSetView",
            "description": "Outbound protocol set inherited by nodes without a dialer override."
          },
          "topology": {
            "$ref": "#/components/schemas/TopologyDefault",
            "description": "Adjacency permission used when no topology rule matches."
          }
        },
        "additionalProperties": false
      },
      "DelegatedSessionResponse": {
        "type": "object",
        "description": "First and only delivery of a delegated session's bearer token.",
        "required": [
          "requestId",
          "user",
          "session",
          "token",
          "client",
          "server"
        ],
        "properties": {
          "client": {
            "type": "string",
            "description": "The client the session was issued to, shown as its label in the account's session list."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "server": {
            "type": [
              "string",
              "null"
            ],
            "description": "Origin at which clients reach this Server, or null when the deployment has not configured one."
          },
          "session": {
            "$ref": "#/components/schemas/SessionMetadata",
            "description": "Identifier and validity period of the delegated session: one hour, not renewable."
          },
          "token": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Opaque bearer credential for `/api/v1`; the session cannot manage the account, other sessions or delegations."
          },
          "user": {
            "$ref": "#/components/schemas/UserView",
            "description": "Account the delegated session acts for."
          }
        },
        "additionalProperties": false
      },
      "DelegationCreated": {
        "type": "object",
        "description": "A one-time authorization code for the client; the Console appends it and the client's own `state` to the redirect target.",
        "required": [
          "requestId",
          "code",
          "expiresAt"
        ],
        "properties": {
          "code": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "The code to deliver as the `code` query parameter; exchangeable once within 60 seconds."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "When the code stops being exchangeable."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "DeleteNodeRequest": {
        "type": "object",
        "description": "Idempotent permanent Node deletion guarded by the expected configuration revision.",
        "required": [
          "requestId",
          "expectedRevision"
        ],
        "properties": {
          "expectedRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Configuration revision required by the compare-and-swap deletion transaction."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Request identifier binding retries to the same caller, network and deletion intent."
          }
        },
        "additionalProperties": false
      },
      "DemandDecision": {
        "oneOf": [
          {
            "type": "object",
            "description": "The demand has authorized complete Mesh paths.",
            "required": [
              "permit",
              "type"
            ],
            "properties": {
              "permit": {
                "$ref": "#/components/schemas/PathPermit",
                "description": "Exact business scope and allowed paths compiled for the demand."
              },
              "type": {
                "type": "string",
                "enum": [
                  "permitted"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Routing selected direct local egress, whose execution requires the matching local-egress permit.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "direct"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Routing selected unmanaged local egress, whose execution requires the matching local-egress permit.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "unmanaged"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Compilation could not produce executable authority for this demand.",
            "required": [
              "reason",
              "type"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/UnavailableReason",
                "description": "Authorization or dependency condition preventing execution."
              },
              "type": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Server compilation result for a business demand; feedback alone does not authorize socket I/O."
      },
      "DemandRequest": {
        "type": "object",
        "description": "Node request for business execution, with the source and network supplied by its authenticated session.",
        "required": [
          "instance",
          "requestId",
          "target",
          "protocol",
          "profile",
          "durationSeconds"
        ],
        "properties": {
          "durationSeconds": {
            "type": "integer",
            "format": "int32",
            "description": "Requested demand lifetime in seconds, from 1 through 300; creation replay does not extend it.",
            "minimum": 0
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current source runtime instance to which the demand is bound."
          },
          "port": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Port",
                "description": "TCP or UDP destination port; null or omission is required for ICMP echo."
              },
              {
                "type": "null"
              }
            ]
          },
          "profile": {
            "type": "string",
            "description": "Authorized routing profile matching the source's current selection."
          },
          "protocol": {
            "$ref": "#/components/schemas/BusinessProtocol",
            "description": "Inner business protocol to authorize."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Stable demand identifier reused with identical content on creation retries."
          },
          "target": {
            "$ref": "#/components/schemas/BusinessTarget",
            "description": "Exact requested business destination."
          }
        },
        "additionalProperties": false
      },
      "DemandResponse": {
        "type": "object",
        "description": "Server compilation feedback requiring configuration synchronization before execution.",
        "required": [
          "publication",
          "decision"
        ],
        "properties": {
          "decision": {
            "$ref": "#/components/schemas/DemandDecision",
            "description": "Current authorization and path result, independent of actual Node installation."
          },
          "publication": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Execution publication associated with the evaluated demand."
          }
        },
        "additionalProperties": false
      },
      "DerivedNodeState": {
        "type": "object",
        "description": "Server-derived IP metadata restricted to endpoint observations visible to the caller.",
        "required": [
          "ipInfo"
        ],
        "properties": {
          "ipInfo": {
            "type": "object",
            "description": "Independent GeoIP and ASN lookups indexed by canonical visible endpoint IP.",
            "additionalProperties": {
              "$ref": "#/components/schemas/IpInfo"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "location": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/GeoObservation",
                "description": "Approximate location from valid eligible local underlay observations, or null when unavailable."
              },
              {
                "type": "null"
              }
            ]
          },
          "locationAbsent": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/LocationAbsentReason",
                "description": "Reason no location can be derived, or null when location is available."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "DeviceAuthorizationView": {
        "type": "object",
        "description": "Signed-in user's view of a device authorization, with approval details filtered by current permissions.",
        "required": [
          "request",
          "approval"
        ],
        "properties": {
          "approval": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/EnrollmentApprovalView",
                "description": "Only the approving user or a current manager of the approved network sees this fragment."
              },
              {
                "type": "null"
              }
            ]
          },
          "request": {
            "$ref": "#/components/schemas/EnrollmentRequestView",
            "description": "Device request and its current lifecycle state."
          }
        },
        "additionalProperties": false
      },
      "DiagnosticDelta": {
        "type": "object",
        "description": "Server-computed keyed changes between two complete execution targets.",
        "required": [
          "remove",
          "upsert"
        ],
        "properties": {
          "remove": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ObservationId"
            },
            "description": "Keys present in the base that must be removed from the reconstructed target."
          },
          "upsert": {
            "type": "object",
            "description": "Complete values to insert or replace; keys cannot also occur in remove.",
            "additionalProperties": {
              "$ref": "#/components/schemas/DiagnosticPermit"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Leased observation task identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "DiagnosticPermit": {
        "type": "object",
        "description": "Server-compiled short-lived measurement authority shared by all Nodes on its allowed complete paths.",
        "required": [
          "observationId",
          "source",
          "instanceId",
          "target",
          "service",
          "paths",
          "replies",
          "expiresAt"
        ],
        "properties": {
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Permit deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339; ordinary instance renewal does not extend it."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Exact source runtime instance authorized to execute the observation."
          },
          "observationId": {
            "$ref": "#/components/schemas/ObservationId",
            "description": "Observation task authorized by this permit."
          },
          "paths": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/LogicalPath"
            },
            "description": "Complete authorized measurement paths preserving every required intermediate Node."
          },
          "replies": {
            "$ref": "#/components/schemas/DiagnosticReplyPolicy",
            "description": "Authentication and path constraint for diagnostic replies."
          },
          "service": {
            "$ref": "#/components/schemas/ObservationServiceScope",
            "description": "Exact authorized traffic counting scope; diagnostic frames are separate\nfrom TCP/UDP/ICMP business packets and create no business flow authority."
          },
          "source": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Only Node authorized to originate the diagnostic requests."
          },
          "target": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Terminal Node authorized to answer the diagnostic requests."
          }
        },
        "additionalProperties": false
      },
      "DiagnosticReplyPolicy": {
        "type": "string",
        "description": "Reply constraint for authenticated diagnostic request and response correlation.",
        "enum": [
          "authenticatedExactReversePath"
        ]
      },
      "DialCandidate": {
        "type": "object",
        "description": "Server-compiled outbound candidate with resolved peer listener parameters and material.",
        "required": [
          "peer",
          "listenerKey",
          "endpoints",
          "priority",
          "parameters"
        ],
        "properties": {
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/DialEndpoint"
            },
            "description": "Configured then Node-reported endpoints; an empty Native UDP list requires coordinated acquisition."
          },
          "listenerKey": {
            "type": "string",
            "description": "Configuration key of the peer listener."
          },
          "parameters": {
            "$ref": "#/components/schemas/DialParameters",
            "description": "Complete protocol-specific client parameters and required material."
          },
          "peer": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Peer accepting this candidate's underlying connection."
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "description": "Configured dial preference, with lower values ordered first.",
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "DialEndpoint": {
        "type": "object",
        "description": "One dialable address of a peer listener together with its provenance.",
        "required": [
          "host",
          "port",
          "source"
        ],
        "properties": {
          "host": {
            "type": "string",
            "description": "Host name or IP address."
          },
          "port": {
            "$ref": "#/components/schemas/Port",
            "description": "Listener port."
          },
          "source": {
            "$ref": "#/components/schemas/DialEndpointSource",
            "description": "Provenance deciding the dial order."
          }
        },
        "additionalProperties": false
      },
      "DialEndpointSource": {
        "type": "string",
        "description": "Where a dial endpoint came from; configured endpoints are tried before Node-reported ones.",
        "enum": [
          "configured",
          "local"
        ]
      },
      "DialParameters": {
        "oneOf": [
          {
            "type": "object",
            "description": "Native UDP connection settings.",
            "required": [
              "options",
              "type"
            ],
            "properties": {
              "options": {
                "$ref": "#/components/schemas/NativeUdpDialer",
                "description": "Selected source node's Native UDP dialer options."
              },
              "type": {
                "type": "string",
                "enum": [
                  "nativeUdp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Native TCP connection settings.",
            "required": [
              "options",
              "type"
            ],
            "properties": {
              "options": {
                "$ref": "#/components/schemas/NativeTcpDialer",
                "description": "Selected source node's Native TCP dialer options."
              },
              "type": {
                "type": "string",
                "enum": [
                  "nativeTcp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "VLESS connection settings with the target's credential and encapsulation.",
            "required": [
              "options",
              "stream",
              "flow",
              "uuid",
              "tls",
              "type"
            ],
            "properties": {
              "flow": {
                "type": "string",
                "description": "Target listener's VLESS flow; only the empty string is supported."
              },
              "options": {
                "$ref": "#/components/schemas/VlessDialer",
                "description": "Selected source node's VLESS dialer options, including any additional WebSocket request headers."
              },
              "stream": {
                "$ref": "#/components/schemas/VlessStream",
                "description": "Concrete target listener encapsulation, `tcp` or `websocket`, after dialer compatibility selection."
              },
              "tls": {
                "$ref": "#/components/schemas/ClientTls",
                "description": "Target-bound TLS verification material and selected client options."
              },
              "type": {
                "type": "string",
                "enum": [
                  "vless"
                ]
              },
              "uuid": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Target listener's explicit or Server-generated, saved UUID credential."
              },
              "websocket": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/ListenerWebSocket",
                    "description": "Target listener's resolved WebSocket path, Host, and response headers; null or omission indicates no WebSocket parameters."
                  },
                  {
                    "type": "null"
                  }
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Trojan TCP CONNECT settings with the target's password and TLS verification material.",
            "required": [
              "options",
              "password",
              "tls",
              "type"
            ],
            "properties": {
              "options": {
                "$ref": "#/components/schemas/TrojanDialer",
                "description": "Selected source node's Trojan dialer options."
              },
              "password": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Target listener's explicit or Server-generated, saved password, preserved without trimming or case normalization."
              },
              "tls": {
                "$ref": "#/components/schemas/ClientTls",
                "description": "Target-bound TLS verification material and selected client options."
              },
              "type": {
                "type": "string",
                "enum": [
                  "trojan"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Protocol-specific connection inputs resolved by the Server for a selected target listener."
      },
      "DialerDelta": {
        "type": "object",
        "description": "Server-computed keyed changes between two complete execution targets.",
        "required": [
          "remove",
          "upsert"
        ],
        "properties": {
          "remove": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Keys present in the base that must be removed from the reconstructed target."
          },
          "upsert": {
            "type": "object",
            "description": "Complete values to insert or replace; keys cannot also occur in remove.",
            "additionalProperties": {
              "$ref": "#/components/schemas/DialCandidate"
            },
            "propertyNames": {
              "type": "string"
            }
          }
        },
        "additionalProperties": false
      },
      "DialerSet": {
        "type": "object",
        "description": "Protocol-keyed outbound selection; an explicit empty object disables active dialing, and a node's explicit set replaces the whole network default set.",
        "properties": {
          "native_tcp": {
            "$ref": "#/components/schemas/NativeTcpDialer",
            "description": "Native TCP options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected."
          },
          "native_udp": {
            "$ref": "#/components/schemas/NativeUdpDialer",
            "description": "Native UDP options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected."
          },
          "trojan": {
            "$ref": "#/components/schemas/TrojanDialer",
            "description": "Trojan options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected."
          },
          "vless": {
            "$ref": "#/components/schemas/VlessDialer",
            "description": "VLESS options; omission excludes this protocol from the selected set, an empty object enables defaults, and null is rejected."
          }
        },
        "additionalProperties": false
      },
      "DialerSetUpdate": {
        "type": "object",
        "description": "Replacement outbound protocol set; omitted protocol entries are removed and `{}` disables active dialing.",
        "properties": {
          "native_tcp": {
            "$ref": "#/components/schemas/NativeTcpDialer",
            "description": "Native TCP options to include; omission excludes this protocol from the replacement set."
          },
          "native_udp": {
            "$ref": "#/components/schemas/NativeUdpDialer",
            "description": "Native UDP options to include; omission excludes this protocol from the replacement set."
          },
          "trojan": {
            "$ref": "#/components/schemas/TrojanDialer",
            "description": "Trojan options to include; omission excludes this protocol from the replacement set."
          },
          "vless": {
            "$ref": "#/components/schemas/VlessDialerUpdate",
            "description": "VLESS options to include, retaining omitted WebSocket secrets within this entry; omission excludes VLESS."
          }
        },
        "additionalProperties": false
      },
      "DialerSetView": {
        "type": "object",
        "description": "Selected outbound protocol set with sensitive values removed.",
        "properties": {
          "native_tcp": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NativeTcpDialer",
                "description": "Native TCP options; null means this protocol is absent from the selected set."
              },
              {
                "type": "null"
              }
            ]
          },
          "native_udp": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NativeUdpDialer",
                "description": "Native UDP options; null means this protocol is absent from the selected set."
              },
              {
                "type": "null"
              }
            ]
          },
          "trojan": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/TrojanDialer",
                "description": "Trojan options; null means this protocol is absent from the selected set."
              },
              {
                "type": "null"
              }
            ]
          },
          "vless": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/VlessDialerView",
                "description": "Redacted VLESS options; null means this protocol is absent from the selected set."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "DialerStream": {
        "type": "string",
        "description": "VLESS target encapsulation selector; defaults to `auto` to use the listener's encapsulation.",
        "enum": [
          "auto",
          "tcp",
          "websocket"
        ]
      },
      "DialerTls": {
        "type": "object",
        "description": "TLS client verification and negotiation options for a target selected by the Server.",
        "properties": {
          "alpn": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Ordered ALPN names of 1–255 bytes without control characters; defaults to empty, and WebSocket permits only `http/1.1` entries."
          },
          "handshakeTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive TLS handshake budget; defaults to `5s` and cannot extend the overall connection or remaining lease budget."
          },
          "maxVersion": {
            "$ref": "#/components/schemas/TlsVersion",
            "description": "Maximum accepted TLS version, `1.2` or `1.3`; defaults to `1.3` and must not precede minVersion."
          },
          "minVersion": {
            "$ref": "#/components/schemas/TlsVersion",
            "description": "Minimum accepted TLS version, `1.2` or `1.3`; defaults to `1.2` and must not exceed maxVersion."
          },
          "trust": {
            "$ref": "#/components/schemas/TlsTrust",
            "description": "Target-bound TLS verification policy; defaults to `managed`, the only accepted value."
          }
        },
        "additionalProperties": false
      },
      "DialerWebSocket": {
        "type": "object",
        "description": "Additional WebSocket client options; the target listener supplies the path and Host.",
        "properties": {
          "headers": {
            "$ref": "#/components/schemas/WebSocketHeaders",
            "description": "Additional request headers, excluding reserved handshake and framing names; defaults to an empty object."
          }
        },
        "additionalProperties": false
      },
      "DialerWebSocketUpdate": {
        "type": "object",
        "description": "WebSocket request header edits that retain omitted sensitive values.",
        "properties": {
          "headers": {
            "$ref": "#/components/schemas/WebSocketHeaders",
            "description": "Omission retains existing extra headers, `{}` clears them, and a nonempty map replaces them without overriding handshake or framing headers."
          }
        },
        "additionalProperties": false
      },
      "Digest": {
        "type": "string",
        "description": "A lossless 32-byte digest represented as exactly 64 lowercase hex digits.\n\nThis value does not compute hashes or define their input encoding or purpose.\nIn particular, serializability does not authorize publication of secret hashes.",
        "maxLength": 64,
        "minLength": 64,
        "pattern": "^[0-9a-f]{64}$"
      },
      "DnsConfiguration": {
        "type": "object",
        "description": "Network naming, upstream resolvers and longest-label-suffix DNS routing.",
        "required": [
          "domain"
        ],
        "properties": {
          "default": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Default resolver keys; defaults to an empty list and must be nonempty when `overrideLocal` is true."
          },
          "domain": {
            "type": "string",
            "description": "Network DNS suffix, normalized as a DNS name and combined with each node's hostname."
          },
          "magicDNS": {
            "type": "boolean",
            "description": "Resolve authorized network node names from configured Mesh addresses; defaults to true."
          },
          "overrideLocal": {
            "type": "boolean",
            "description": "Use default resolvers for ordinary names; defaults to false, preserving system resolution outside MagicDNS and split routes without an exit."
          },
          "routes": {
            "type": "object",
            "description": "DNS suffixes mapped to nonempty resolver-key lists; the longest matching label-boundary suffix wins.",
            "additionalProperties": {
              "type": "array",
              "items": {
                "type": "string"
              }
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "searchDomains": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Additional search suffixes in priority order; defaults to an empty list, with the MagicDNS domain prepended when enabled."
          },
          "servers": {
            "type": "object",
            "description": "Upstream resolvers keyed by the names used in `default` and `routes`; defaults to an empty map.",
            "additionalProperties": {
              "$ref": "#/components/schemas/DnsServer"
            },
            "propertyNames": {
              "type": "string"
            }
          }
        },
        "additionalProperties": false
      },
      "DnsEgress": {
        "oneOf": [
          {
            "type": "object",
            "description": "Compilation proved that the DNS selection has no exit possibility.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "noExit"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "DNS egress lacks a required executable plan.",
            "required": [
              "reason",
              "type"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/DnsEgressUnavailable",
                "description": "Condition preventing DNS egress execution."
              },
              "type": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Server evaluation of whether DNS routing requires an explicit exit plan."
      },
      "DnsEgressUnavailable": {
        "type": "string",
        "description": "Missing execution dependency for DNS egress.",
        "enum": [
          "exitPlanRequired"
        ]
      },
      "DnsExecution": {
        "type": "object",
        "description": "DNS settings and trusted name bindings compiled for one Node's execution target.",
        "required": [
          "settings",
          "meshHosts",
          "associations",
          "profile",
          "egress"
        ],
        "properties": {
          "associations": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TrustedDnsAssociation"
            },
            "description": "Current trusted DNS associations included for this source's domain demands."
          },
          "egress": {
            "$ref": "#/components/schemas/DnsEgress",
            "description": "Proved absence of exit selection only; never permission to open a socket."
          },
          "meshHosts": {
            "type": "object",
            "description": "Configured Mesh names and addresses filtered by this Node's source Access permissions.",
            "additionalProperties": {
              "$ref": "#/components/schemas/NodeAddresses"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "profile": {
            "type": "string",
            "description": "Regular resolution follows this selection; explicit resolver exceptions retain useWithExit."
          },
          "settings": {
            "$ref": "#/components/schemas/DnsConfiguration",
            "description": "DNS configuration copied from the publication's source revision."
          }
        },
        "additionalProperties": false
      },
      "DnsServer": {
        "type": "object",
        "description": "An explicit upstream resolver identified by its literal IP address.",
        "required": [
          "type",
          "server"
        ],
        "properties": {
          "server": {
            "type": "string",
            "description": "Literal IPv4 or IPv6 resolver address, avoiding a DNS bootstrap dependency."
          },
          "serverPort": {
            "$ref": "#/components/schemas/Port",
            "description": "Resolver destination port in 1–65535; defaults to 53."
          },
          "type": {
            "$ref": "#/components/schemas/DnsTransport",
            "description": "UDP or TCP transport for upstream DNS queries."
          },
          "useWithExit": {
            "type": "boolean",
            "description": "Retain this resolver's explicit DNS role when an exit is selected; defaults to false."
          }
        },
        "additionalProperties": false
      },
      "DnsTransport": {
        "type": "string",
        "description": "Transport used to send queries to an explicitly configured DNS resolver.",
        "enum": [
          "udp",
          "tcp"
        ]
      },
      "EgressCapturePlan": {
        "type": "object",
        "description": "Packet ownership only. These disjoint CIDRs grant no business Access.",
        "required": [
          "generation",
          "managedIpv4",
          "defaultCapture",
          "directIpv4",
          "unmanagedIpv4",
          "sinkIpv6"
        ],
        "properties": {
          "defaultCapture": {
            "type": "boolean",
            "description": "True for default IPv4 capture, including a first-match exit covering all IPv4."
          },
          "directIpv4": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Explicit direct IPv4 exceptions after first-match subtraction."
          },
          "generation": {
            "$ref": "#/components/schemas/Digest",
            "description": "Identity of the effective selection and its routing inputs."
          },
          "managedIpv4": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Disjoint IPv4 ranges handed to the unique data-plane owner."
          },
          "sinkIpv6": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "IPv6 exit ranges that must be sunk; Mesh routes take precedence."
          },
          "unavailable": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/UnavailableReason",
                "description": "Unresolved routing requires a fail-closed boundary, not local fallback."
              },
              {
                "type": "null"
              }
            ]
          },
          "unmanagedIpv4": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Explicit unmanaged IPv4 exceptions after first-match subtraction."
          }
        },
        "additionalProperties": false
      },
      "EgressRouting": {
        "type": "object",
        "description": "Public egress rules, named profiles and authorization-constrained profile selection.",
        "properties": {
          "assignments": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ProfileAssignment"
            },
            "description": "Node-selector profile assignments; defaults to an empty list."
          },
          "auto": {
            "$ref": "#/components/schemas/AutoProfile",
            "description": "Region-based profile selector; omission leaves `auto` undefined."
          },
          "profiles": {
            "type": "object",
            "description": "Administrator-defined profiles keyed by nonreserved names; defaults to an empty map.",
            "additionalProperties": {
              "$ref": "#/components/schemas/RoutingProfile"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "rules": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/EgressRule"
            },
            "description": "First-match public rules evaluated before every selected profile; defaults to an empty list."
          }
        },
        "additionalProperties": false
      },
      "EgressRule": {
        "type": "object",
        "description": "Ordered destination rule evaluated before the selected profile's fallback.",
        "required": [
          "match",
          "action"
        ],
        "properties": {
          "action": {
            "$ref": "#/components/schemas/RuleAction",
            "description": "Action used when this rule is the first match, without falling through on path failure."
          },
          "match": {
            "$ref": "#/components/schemas/RouteMatch",
            "description": "Destination conditions combined by union, requiring at least one nonempty list."
          },
          "remark": {
            "type": "string",
            "description": "Free-form description, defaulting to an empty string; duplicate descriptions are allowed."
          }
        },
        "additionalProperties": false
      },
      "EmptyParams": {
        "type": "object",
        "description": "Empty parameter object for problem codes that expose no additional context.",
        "additionalProperties": false
      },
      "Endpoint": {
        "type": "object",
        "description": "A host and port identifying a transport endpoint, without asserting reachability.",
        "required": [
          "host",
          "port"
        ],
        "properties": {
          "host": {
            "type": "string",
            "description": "IP address or DNS name, without a URL scheme, path, or port suffix."
          },
          "port": {
            "$ref": "#/components/schemas/Port",
            "description": "Endpoint port in the range 1–65535."
          }
        },
        "additionalProperties": false
      },
      "EndpointCandidate": {
        "type": "object",
        "description": "Node-reported endpoint candidate bound to its original source and validity interval.",
        "required": [
          "address",
          "origin",
          "observedAt",
          "expiresAt"
        ],
        "properties": {
          "address": {
            "type": "string",
            "description": "Host and port; interface and STUN candidates require an IP literal with a port."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC deadline after which this candidate observation is no longer current."
          },
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC observation time, or the time a configured association was checked."
          },
          "origin": {
            "$ref": "#/components/schemas/EndpointSource",
            "description": "Source evidence for this candidate, not proof of reachability."
          }
        },
        "additionalProperties": false
      },
      "EndpointKind": {
        "type": "string",
        "description": "Coarse provenance only; private observer and interface details are excluded.",
        "enum": [
          "configured",
          "interface",
          "stun"
        ]
      },
      "EndpointSource": {
        "oneOf": [
          {
            "type": "object",
            "description": "The endpoint was explicitly supplied in the configuration.",
            "required": [
              "configPath",
              "source"
            ],
            "properties": {
              "configPath": {
                "type": "string",
                "description": "Path to the matching endpoint declaration in the stored configuration."
              },
              "source": {
                "type": "string",
                "enum": [
                  "configured"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The Node obtained a usable address from a local interface.",
            "required": [
              "interface",
              "source"
            ],
            "properties": {
              "interface": {
                "type": "string",
                "description": "Local interface from which the address was obtained."
              },
              "source": {
                "type": "string",
                "enum": [
                  "interface"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A STUN exchange observed the mapping of this UDP socket.",
            "required": [
              "observer",
              "source"
            ],
            "properties": {
              "observer": {
                "type": "string",
                "description": "STUN observer used for the mapping observation."
              },
              "source": {
                "type": "string",
                "enum": [
                  "stun"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Origin of an endpoint candidate, without asserting peer reachability."
      },
      "EnrollmentApprovalView": {
        "type": "object",
        "description": "Approving user and selected network for a device enrollment.",
        "required": [
          "userId",
          "networkId"
        ],
        "properties": {
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network selected by the approving user and rechecked at redemption."
          },
          "userId": {
            "$ref": "#/components/schemas/UserId",
            "description": "User whose approval authorizes the device enrollment."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentDeviceProof": {
        "type": "object",
        "description": "Device possession proof bound to the original enrollment request and PKCE challenge.",
        "required": [
          "verifier",
          "signature"
        ],
        "properties": {
          "signature": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Device control-key signature for the requested enrollment operation."
          },
          "verifier": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Private PKCE verifier matching the original intent's challenge."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentIntent": {
        "type": "object",
        "description": "Signed device enrollment intent binding public keys, PKCE and any existing registration.",
        "required": [
          "id",
          "controlKey",
          "dataplaneKey",
          "codeChallenge",
          "hostname",
          "existing"
        ],
        "properties": {
          "codeChallenge": {
            "type": "string",
            "description": "PKCE challenge binding later redemption to the device's private verifier."
          },
          "controlKey": {
            "$ref": "#/components/schemas/NodePublicKey",
            "description": "Device's persistent Ed25519 control public key."
          },
          "dataplaneKey": {
            "$ref": "#/components/schemas/NodePublicKey",
            "description": "Independent X25519 public key for data-plane protection."
          },
          "existing": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ExistingNodeBinding",
                "description": "Always present in the signed intent; a first enrollment uses explicit null."
              },
              {
                "type": "null"
              }
            ]
          },
          "hostname": {
            "type": "string",
            "description": "Device-provided hostname used as the initial naming input."
          },
          "id": {
            "$ref": "#/components/schemas/EnrollmentRequestId",
            "description": "Device-generated identifier for this enrollment attempt."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentKeyActions": {
        "type": "object",
        "description": "Current caller's permitted enrollment key operations, rechecked on mutation.",
        "required": [
          "revoke"
        ],
        "properties": {
          "revoke": {
            "type": "boolean",
            "description": "Whether the caller may currently revoke this key."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentKeyId": {
        "type": "string",
        "format": "uuid",
        "description": "One-use enrollment key identifier, not the secret.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "EnrollmentKeyResponse": {
        "type": "object",
        "description": "Enrollment key creation result with one-time secret delivery.",
        "required": [
          "key",
          "secretDelivery"
        ],
        "properties": {
          "key": {
            "$ref": "#/components/schemas/EnrollmentKeyView",
            "description": "Created key's non-secret metadata, also returned on an idempotent retry."
          },
          "secret": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/WireSecret",
                "description": "Raw enrollment credential on first delivery, or null when delivery is unavailable."
              },
              {
                "type": "null"
              }
            ]
          },
          "secretDelivery": {
            "$ref": "#/components/schemas/SecretDelivery",
            "description": "Whether this response includes the newly created secret."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentKeyView": {
        "type": "object",
        "description": "Non-secret enrollment key metadata visible to authorized network administrators.",
        "required": [
          "id",
          "tags",
          "createdBy",
          "initialSettings",
          "actions",
          "nodeId",
          "consumedAt",
          "createdAt",
          "expiresAt",
          "revokedAt"
        ],
        "properties": {
          "actions": {
            "$ref": "#/components/schemas/EnrollmentKeyActions",
            "description": "Current caller's eligibility to operate on this key."
          },
          "consumedAt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Timestamp",
                "description": "UTC consumption time, or null if the key has not been consumed."
              },
              {
                "type": "null"
              }
            ]
          },
          "consumedRequestId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/EnrollmentRequestId",
                "description": "Enrollment request that consumed this key, or null before consumption."
              },
              {
                "type": "null"
              }
            ]
          },
          "createdAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC time at which the key was created."
          },
          "createdBy": {
            "$ref": "#/components/schemas/UserId",
            "description": "User who created the enrollment key."
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC deadline after which the key cannot be consumed."
          },
          "id": {
            "$ref": "#/components/schemas/EnrollmentKeyId",
            "description": "Stable identifier of the enrollment key record."
          },
          "initialSettings": {
            "$ref": "#/components/schemas/InitialNodeSettingsView",
            "description": "Non-secret summary of the initial Node configuration."
          },
          "nodeId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NodeId",
                "description": "Node registered by this key, or null before consumption."
              },
              {
                "type": "null"
              }
            ]
          },
          "revokedAt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Timestamp",
                "description": "UTC revocation time, or null if the key has not been revoked."
              },
              {
                "type": "null"
              }
            ]
          },
          "tags": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Tags assigned to the Node when this key is consumed."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentKeysResponse": {
        "type": "object",
        "description": "Newest-first enrollment key page under the caller's current network permissions.",
        "required": [
          "items",
          "total",
          "nextCursor",
          "asOf"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "UTC snapshot time retained across continuation pages."
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/EnrollmentKeyView"
            },
            "description": "Non-secret enrollment key records in this page."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Continuation cursor for the same bounded listing, or null on the final page."
          },
          "total": {
            "$ref": "#/components/schemas/Count",
            "description": "Number of matching keys within the initial page's creation upper bound."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentPhase": {
        "type": "string",
        "description": "Server-evaluated lifecycle of a device enrollment request.",
        "enum": [
          "pending",
          "approved",
          "withdrawn",
          "cancelled",
          "consumed",
          "expired"
        ]
      },
      "EnrollmentPollResponse": {
        "type": "object",
        "description": "Enrollment outcome delivered only after validating the requesting device's proof.",
        "required": [
          "request",
          "approval"
        ],
        "properties": {
          "approval": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/EnrollmentApprovalView",
                "description": "Available only after the device proves possession of this request."
              },
              {
                "type": "null"
              }
            ]
          },
          "code": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/WireSecret",
                "description": "One-time redemption code when an approved result is deliverable, or null otherwise."
              },
              {
                "type": "null"
              }
            ]
          },
          "nodeId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NodeId",
                "description": "Registered Node after consumption, or null before the registration result exists."
              },
              {
                "type": "null"
              }
            ]
          },
          "request": {
            "$ref": "#/components/schemas/EnrollmentRequestView",
            "description": "Enrollment request's current metadata and phase."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentRequestId": {
        "type": "string",
        "format": "uuid",
        "description": "Device authorization request identifier, not a credential.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "EnrollmentRequestView": {
        "type": "object",
        "description": "Public device enrollment request details and its current lifecycle phase.",
        "required": [
          "id",
          "hostname",
          "controlKey",
          "existing",
          "returnMethod",
          "phase",
          "expiresAt",
          "authorizationUrl"
        ],
        "properties": {
          "authorizationUrl": {
            "type": [
              "string",
              "null"
            ],
            "description": "Where a signed-in user authorizes this device, built by the Server from\nits Console origin; null when the deployment has not configured one."
          },
          "controlKey": {
            "$ref": "#/components/schemas/NodePublicKey",
            "description": "Public control identity whose possession the device proved."
          },
          "existing": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ExistingNodeBinding",
                "description": "Existing registration being reauthorized, or null for first enrollment."
              },
              {
                "type": "null"
              }
            ]
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC deadline for completing this enrollment request."
          },
          "hostname": {
            "type": "string",
            "description": "Hostname supplied by the requesting device."
          },
          "id": {
            "$ref": "#/components/schemas/EnrollmentRequestId",
            "description": "Identifier of the device's enrollment request."
          },
          "phase": {
            "$ref": "#/components/schemas/EnrollmentPhase",
            "description": "Current Server-evaluated request lifecycle phase."
          },
          "returnMethod": {
            "$ref": "#/components/schemas/EnrollmentReturnMethod",
            "description": "Mechanism by which the device retrieves the authorization outcome."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentResponse": {
        "type": "object",
        "description": "Committed Node registration and public key bindings returned to the enrolling device.",
        "required": [
          "controllerId",
          "networkId",
          "nodeId",
          "keys"
        ],
        "properties": {
          "controllerId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Durable Server installation identity to pin alongside the HTTPS origin."
          },
          "keys": {
            "$ref": "#/components/schemas/NodeKeyMetadata",
            "description": "Public control and data-plane key bindings committed by enrollment."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network to which the device is now registered."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Stable registered Node identity."
          }
        },
        "additionalProperties": false
      },
      "EnrollmentReturnMethod": {
        "type": "string",
        "description": "Supported mechanism for delivering an enrollment outcome to the device.",
        "enum": [
          "poll"
        ]
      },
      "EntityRemovals": {
        "type": "object",
        "description": "Explicit entity removals applied separately from entity replacements.",
        "required": [
          "nodes",
          "members"
        ],
        "properties": {
          "members": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MembershipId"
            },
            "description": "Membership identifiers to remove from the entity inventory."
          },
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Node identifiers to remove from the entity inventory."
          }
        },
        "additionalProperties": false
      },
      "ErrorResponse": {
        "type": "object",
        "description": "HTTP failure envelope with a stable problem code and safe fallback text.",
        "required": [
          "requestId",
          "status",
          "problem"
        ],
        "properties": {
          "problem": {
            "$ref": "#/components/schemas/Problem",
            "description": "Typed failure whose code determines the available parameters."
          },
          "requestId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/RequestId",
                "description": "Null only when secure request-ID generation itself is unavailable."
              },
              {
                "type": "null"
              }
            ]
          },
          "status": {
            "type": "integer",
            "format": "int32",
            "description": "HTTP status code of the failed response.",
            "maximum": 65535,
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "EvaluateConfigurationRequest": {
        "type": "object",
        "description": "Read-only path planning request for one source and destination.",
        "required": [
          "configuration",
          "source",
          "target"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/EvaluationConfiguration",
            "description": "Current configuration or a draft that must satisfy strict revision checks."
          },
          "profile": {
            "type": [
              "string",
              "null"
            ],
            "description": "Omitted: the source's current selection or default."
          },
          "source": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Source node, required to exist in the evaluated configuration."
          },
          "target": {
            "$ref": "#/components/schemas/EvaluationTarget",
            "description": "Business destination used for access and routing evaluation."
          }
        },
        "additionalProperties": false
      },
      "EvaluatedDestination": {
        "oneOf": [
          {
            "type": "object",
            "description": "A configured node, named directly or through its Mesh address.",
            "required": [
              "nodeId",
              "type"
            ],
            "properties": {
              "nodeId": {
                "$ref": "#/components/schemas/NodeId"
              },
              "type": {
                "type": "string",
                "enum": [
                  "node"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "An address that is not a node's: an unassigned Mesh pool address, a\nprivate/non-public range, or the public Internet.",
            "required": [
              "address",
              "class",
              "type"
            ],
            "properties": {
              "address": {
                "type": "string"
              },
              "class": {
                "$ref": "#/components/schemas/AddressClass"
              },
              "type": {
                "type": "string",
                "enum": [
                  "address"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A DNS name; which addresses it resolves to is unknown to the plan.",
            "required": [
              "name",
              "type"
            ],
            "properties": {
              "name": {
                "type": "string"
              },
              "type": {
                "type": "string",
                "enum": [
                  "domain"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A public destination no rule names: only fallbacks apply.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "internet"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "What the request's target resolved to in the evaluated configuration."
      },
      "Evaluation": {
        "type": "object",
        "description": "The plan for one source and destination; see the path evaluation reference.",
        "required": [
          "source",
          "destination",
          "profile",
          "access",
          "route",
          "targets",
          "paths",
          "outcome"
        ],
        "properties": {
          "access": {
            "$ref": "#/components/schemas/AccessEvaluation"
          },
          "destination": {
            "$ref": "#/components/schemas/EvaluatedDestination"
          },
          "outcome": {
            "$ref": "#/components/schemas/EvaluationOutcome"
          },
          "paths": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PlannedPath"
            },
            "description": "Planned paths across all targets, the primary path first."
          },
          "profile": {
            "$ref": "#/components/schemas/ProfileEvaluation"
          },
          "route": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/RouteEvaluation",
                "description": "Null when no route decision applies: an unassigned Mesh address, or a\nremote rule set whose content is not available yet."
              },
              {
                "type": "null"
              }
            ]
          },
          "source": {
            "$ref": "#/components/schemas/NodeId"
          },
          "targets": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TargetEvaluation"
            },
            "description": "Every target the route names, with its problem or path count."
          }
        },
        "additionalProperties": false
      },
      "EvaluationConfiguration": {
        "oneOf": [
          {
            "type": "object",
            "description": "Evaluate the current saved network configuration.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "current"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Evaluate a full JSONC draft without saving it or generating permanent materials.",
            "required": [
              "expectedRevision",
              "sourceJsonc",
              "type"
            ],
            "properties": {
              "expectedRevision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Current revision the draft must match; a stale baseline returns `revision_conflict`."
              },
              "sourceJsonc": {
                "$ref": "#/components/schemas/WireSecret",
                "description": "Complete sensitive JSONC draft to validate and evaluate."
              },
              "type": {
                "type": "string",
                "enum": [
                  "document"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Evaluate ordered typed edits against the current baseline without committing them.",
            "required": [
              "expectedRevision",
              "ops",
              "type"
            ],
            "properties": {
              "expectedRevision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Current revision the draft must match; a stale baseline returns `revision_conflict`."
              },
              "ops": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/ConfigurationOperation"
                },
                "description": "Ordered, individually authorized edits that form the evaluated target."
              },
              "type": {
                "type": "string",
                "enum": [
                  "operations"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Current configuration or a CAS-bound draft used for read-only path planning."
      },
      "EvaluationOutcome": {
        "oneOf": [
          {
            "type": "object",
            "description": "Paths exist and access allows the traffic; `paths` lists them, primary first.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "permitted"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "direct"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "unmanaged"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "required": [
              "reason",
              "type"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/UnavailableReason"
              },
              "type": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "What a demand from the source to the destination would get, as compilation decides it."
      },
      "EvaluationResponse": {
        "type": "object",
        "description": "Rule and path planning result, independent of live connectivity or installation state.",
        "required": [
          "requestId",
          "revision",
          "draft",
          "evaluation"
        ],
        "properties": {
          "draft": {
            "type": "boolean",
            "description": "Whether the result evaluates proposed changes rather than the current saved configuration."
          },
          "evaluation": {
            "$ref": "#/components/schemas/Evaluation",
            "description": "Resolved destination, grants, routing decisions, planned paths and availability outcome."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for this HTTP request."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "The current version the plan is based on; a draft is evaluated as a\nchange to it."
          }
        },
        "additionalProperties": false
      },
      "EvaluationTarget": {
        "oneOf": [
          {
            "type": "object",
            "description": "Plan traffic to a concrete Mesh node.",
            "required": [
              "nodeId",
              "type"
            ],
            "properties": {
              "nodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Destination node in the evaluated network configuration."
              },
              "type": {
                "type": "string",
                "enum": [
                  "node"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Plan traffic to a literal IP, resolving allocated Mesh addresses to their nodes.",
            "required": [
              "address",
              "type"
            ],
            "properties": {
              "address": {
                "type": "string",
                "description": "Literal IPv4 or IPv6 destination address."
              },
              "type": {
                "type": "string",
                "enum": [
                  "address"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Plan name-based routing without performing DNS resolution.",
            "required": [
              "name",
              "type"
            ],
            "properties": {
              "name": {
                "type": "string",
                "description": "Exact DNS name, normalized for matching against domain and label-suffix rules."
              },
              "type": {
                "type": "string",
                "enum": [
                  "domain"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Represent an unnamed public destination using Internet grants and the profile fallback.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "internet"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Business destination to plan without inferring DNS context from an IP address."
      },
      "EventActor": {
        "oneOf": [
          {
            "type": "object",
            "description": "Authenticated user responsible for the change.",
            "required": [
              "userId",
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "user"
                ]
              },
              "userId": {
                "$ref": "#/components/schemas/UserId",
                "description": "Stable account identifier of the acting user."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Authenticated Node responsible for the change.",
            "required": [
              "nodeId",
              "type"
            ],
            "properties": {
              "nodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Stable identifier of the acting Node."
              },
              "type": {
                "type": "string",
                "enum": [
                  "node"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Server-originated change without a user or Node actor.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "system"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Actor identity withheld because the subscriber lacks management scope.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "redacted"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Actor reference filtered for the subscriber's current disclosure permissions."
      },
      "EventId": {
        "type": "string",
        "format": "uuid",
        "description": "Source observation event identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "ExchangeDelegationRequest": {
        "type": "object",
        "description": "A delegation client's exchange of an authorization code for a session.",
        "required": [
          "client",
          "code",
          "codeVerifier",
          "redirectUri"
        ],
        "properties": {
          "client": {
            "type": "string",
            "description": "The client name the code was issued for."
          },
          "code": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "The `code` received in the redirect."
          },
          "codeVerifier": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "The PKCE verifier whose S256 challenge was sent when authorizing; 43–128 unreserved characters."
          },
          "redirectUri": {
            "type": "string",
            "description": "The exact `redirectUri` used when authorizing."
          }
        },
        "additionalProperties": false
      },
      "ExecutionAddress": {
        "type": "string",
        "description": "Numeric IPv4 or IPv6 business destination used in an execution binding."
      },
      "ExistingNodeBinding": {
        "type": "object",
        "description": "Current registered identity that a device must prove when enrolling again.",
        "required": [
          "networkId",
          "nodeId",
          "keyId",
          "keyGeneration"
        ],
        "properties": {
          "keyGeneration": {
            "$ref": "#/components/schemas/Version",
            "description": "Current control-key generation required by the existing binding."
          },
          "keyId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Currently bound control key that must match the enrollment public key."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Existing network binding, which approval cannot replace with another network."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Existing Node identity retained by re-enrollment."
          }
        },
        "additionalProperties": false
      },
      "ExitCapability": {
        "type": "object",
        "description": "Target-bound exit capability observation, accepted before applied as well as after it.",
        "required": [
          "instanceId",
          "target",
          "supportedFamilies",
          "installedFamilies",
          "status",
          "observedAt"
        ],
        "properties": {
          "installedFamilies": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ExitFamily"
            },
            "description": "Families actually installed; ready alone does not imply installed."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Runtime incarnation which made the observation."
          },
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original observation time, preserved on retries."
          },
          "status": {
            "$ref": "#/components/schemas/ExitStatus",
            "description": "Actual prerequisite or installation result."
          },
          "supportedFamilies": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ExitFamily"
            },
            "description": "Families supported by the running backend."
          },
          "target": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact execution target whose role was probed."
          }
        },
        "additionalProperties": false
      },
      "ExitDestinationScope": {
        "oneOf": [
          {
            "type": "object",
            "description": "Exact target must also belong to Policy's public Internet set.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "internet"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Explicit CIDR Access covers the exact target, including authorized LAN.",
            "required": [
              "prefix",
              "type"
            ],
            "properties": {
              "prefix": {
                "type": "string",
                "description": "Administrator-authorized CIDR, not an automatically inferred LAN."
              },
              "type": {
                "type": "string",
                "enum": [
                  "cidr"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Why an exact external destination is eligible; never widens PathPermit.target."
      },
      "ExitFamily": {
        "type": "string",
        "description": "Inner address family, independent of the outer carrier.",
        "enum": [
          "ipv4",
          "ipv6"
        ]
      },
      "ExitNat": {
        "type": "string",
        "description": "Translation mode allowed for an exit role.",
        "enum": [
          "masquerade"
        ]
      },
      "ExitPermit": {
        "type": "object",
        "description": "Additional constraints on an external PathPermit; paths name its exit terminals.",
        "required": [
          "generation",
          "family",
          "destination",
          "roles"
        ],
        "properties": {
          "destination": {
            "$ref": "#/components/schemas/ExitDestinationScope",
            "description": "Destination classification checked again by packet admission."
          },
          "family": {
            "$ref": "#/components/schemas/ExitFamily",
            "description": "Source address family; initial implementation permits IPv4 only."
          },
          "generation": {
            "$ref": "#/components/schemas/Digest",
            "description": "Changes on every effective profile selection, including switching back."
          },
          "roles": {
            "type": "object",
            "description": "Every eligible terminal bound to the role generation it must install.",
            "additionalProperties": {
              "$ref": "#/components/schemas/ExitRolePlan"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "ExitRolePlan": {
        "type": "object",
        "description": "Resource preparation responsibility; never a grant to forward a packet.\nNetwork, node, publication and deadlines come from the enclosing target/lease.",
        "required": [
          "instanceId",
          "generation",
          "family",
          "nat"
        ],
        "properties": {
          "family": {
            "$ref": "#/components/schemas/ExitFamily",
            "description": "Inner family implemented by this role."
          },
          "generation": {
            "$ref": "#/components/schemas/Digest",
            "description": "Role identity stable across lease renewal; changes with source revision."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Current instance which may install the role."
          },
          "nat": {
            "$ref": "#/components/schemas/ExitNat",
            "description": "Required translation mode."
          }
        },
        "additionalProperties": false
      },
      "ExitStatus": {
        "oneOf": [
          {
            "type": "object",
            "description": "No exit role has been requested.",
            "required": [
              "status"
            ],
            "properties": {
              "status": {
                "type": "string",
                "enum": [
                  "notRequested"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Prerequisites are being tested; not ready.",
            "required": [
              "status"
            ],
            "properties": {
              "status": {
                "type": "string",
                "enum": [
                  "probing"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The reported families completed prerequisite checks.",
            "required": [
              "status"
            ],
            "properties": {
              "status": {
                "type": "string",
                "enum": [
                  "ready"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Installation or prerequisite checks failed.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/ExitUnavailableReason",
                "description": "Concrete failure; never replaced with a successful observation."
              },
              "status": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Observed exit lifecycle; unavailable carries a mandatory typed cause."
      },
      "ExitUnavailableReason": {
        "type": "string",
        "description": "Concrete prerequisite preventing exit installation or execution.",
        "enum": [
          "ipForwardDisabled",
          "forwardingProbeFailed",
          "netAdminMissing",
          "nftUnavailable",
          "conntrackUnavailable",
          "reversePathFilterConflict",
          "tunUnavailable",
          "firewallConflict",
          "routeConflict",
          "dnsUnavailable",
          "unsupportedFamily",
          "ownershipConflict"
        ]
      },
      "Freshness": {
        "type": "string",
        "description": "Server-derived validity of the latest report under current identity, instance and configuration state.",
        "enum": [
          "current",
          "awaiting_report",
          "expired",
          "superseded",
          "config_mismatch",
          "revoked"
        ]
      },
      "GeoIpData": {
        "type": "object",
        "description": "Approximate geographic metadata returned by the Server's IP dataset.",
        "properties": {
          "city": {
            "type": [
              "string",
              "null"
            ],
            "description": "Dataset-provided city name, or null if absent."
          },
          "country": {
            "type": [
              "string",
              "null"
            ],
            "description": "Dataset-provided ISO 3166-1 alpha-2 country or territory code, or null if absent."
          },
          "position": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/GeoPosition",
                "description": "Approximate coordinates supplied by the dataset, or null when none are available."
              },
              {
                "type": "null"
              }
            ]
          },
          "region": {
            "type": [
              "string",
              "null"
            ],
            "description": "Dataset-provided first-level region name or identifier, or null if absent."
          }
        },
        "additionalProperties": false
      },
      "GeoObservation": {
        "type": "object",
        "description": "Server-derived approximate location retaining the selected local observation's source and deadline.",
        "required": [
          "address",
          "sourceSocketId",
          "sourceGeneration",
          "datasetId",
          "data",
          "endpointSource",
          "observedAt",
          "expiresAt"
        ],
        "properties": {
          "address": {
            "type": "string",
            "description": "Eligible observed public underlay IP used for the geographic lookup."
          },
          "data": {
            "$ref": "#/components/schemas/GeoIpData",
            "description": "Geographic metadata returned for the selected observed address."
          },
          "datasetId": {
            "$ref": "#/components/schemas/ArtifactId",
            "description": "Immutable geographic dataset artifact used to derive the location."
          },
          "endpointSource": {
            "$ref": "#/components/schemas/LocationSource",
            "description": "Local observation source; configured declarations alone cannot establish a Node location."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC endpoint deadline limiting this location's current validity."
          },
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC time of the endpoint observation, not the lookup time."
          },
          "sourceGeneration": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Source socket resource generation associated with the observation."
          },
          "sourceSocketId": {
            "type": "string",
            "description": "Actual socket whose endpoint observation supplies the address."
          }
        },
        "additionalProperties": false
      },
      "GeoPosition": {
        "type": "object",
        "description": "Dataset-provided approximate coordinates and optional accuracy radius.",
        "required": [
          "latitude",
          "longitude"
        ],
        "properties": {
          "accuracyRadiusKm": {
            "type": [
              "number",
              "null"
            ],
            "format": "double",
            "description": "Dataset provider's nonnegative estimated accuracy radius in kilometers, or null if absent.",
            "minimum": 0
          },
          "latitude": {
            "type": "number",
            "format": "double",
            "description": "Finite latitude in degrees, from -90 through 90.",
            "maximum": 90,
            "minimum": -90
          },
          "longitude": {
            "type": "number",
            "format": "double",
            "description": "Finite longitude in degrees, from -180 through 180.",
            "maximum": 180,
            "minimum": -180
          }
        },
        "additionalProperties": false
      },
      "HistoricalEvent": {
        "type": "object",
        "description": "Stored event with authenticated source identity and original Server reception time.",
        "required": [
          "sourceNodeId",
          "instanceId",
          "receivedAt",
          "event"
        ],
        "properties": {
          "event": {
            "$ref": "#/components/schemas/ObservationEvent",
            "description": "Original event facts, occurrence time and execution target."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Runtime instance under which the event was reported."
          },
          "receivedAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC time at which the Server first accepted the event."
          },
          "sourceNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Authenticated Node that reported the event."
          }
        },
        "additionalProperties": false
      },
      "HistoryAccepted": {
        "type": "object",
        "description": "Server acceptance counts for an idempotent historical event batch.",
        "required": [
          "accepted",
          "duplicates"
        ],
        "properties": {
          "accepted": {
            "type": "integer",
            "format": "int32",
            "description": "Number of newly accepted events in the batch.",
            "minimum": 0
          },
          "duplicates": {
            "type": "integer",
            "format": "int32",
            "description": "Number of events already accepted with the same source, identifier and content.",
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "HistoryBucket": {
        "type": "object",
        "description": "Aggregate of real probe results sharing a source, execution context and measurement scope.",
        "required": [
          "from",
          "to",
          "sourceNodeId",
          "sampleCount",
          "success",
          "timeout",
          "failed",
          "instanceId",
          "publicationId",
          "path",
          "kind"
        ],
        "properties": {
          "failed": {
            "type": "integer",
            "format": "int32",
            "description": "Number of measured operations that failed.",
            "minimum": 0
          },
          "from": {
            "type": "string",
            "format": "date-time",
            "description": "UTC start of the aggregation bucket."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Source instance shared by the aggregated measurements."
          },
          "kind": {
            "$ref": "#/components/schemas/ProbeKind",
            "description": "Measured operation kind, preserving its timing scope."
          },
          "linkId": {
            "type": [
              "string",
              "null"
            ],
            "description": "Shared channel identifier, or null when the measurements have none."
          },
          "meanSuccessDurationUs": {
            "type": [
              "number",
              "null"
            ],
            "format": "double",
            "description": "Mean available duration of successful operations in microseconds, or null without timed successes."
          },
          "path": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Ordered path shared by the aggregated measurements."
          },
          "publicationId": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Execution publication shared by the aggregated measurements."
          },
          "sampleCount": {
            "type": "integer",
            "format": "int32",
            "description": "Number of real probe results contributing to the bucket.",
            "minimum": 0
          },
          "sourceNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that produced the aggregated measurements."
          },
          "success": {
            "type": "integer",
            "format": "int32",
            "description": "Number of successful measured operations.",
            "minimum": 0
          },
          "timeout": {
            "type": "integer",
            "format": "int32",
            "description": "Number of measured operations that timed out.",
            "minimum": 0
          },
          "to": {
            "type": "string",
            "format": "date-time",
            "description": "UTC end of the aggregation bucket."
          },
          "transport": {
            "type": [
              "string",
              "null"
            ],
            "description": "Shared transport, or null when the measurements have none."
          }
        },
        "additionalProperties": false
      },
      "HistoryCoverage": {
        "oneOf": [
          {
            "type": "object",
            "description": "The Server pruned retained history for this source before a known boundary.",
            "required": [
              "sourceNodeId",
              "before",
              "reason",
              "type"
            ],
            "properties": {
              "before": {
                "type": "string",
                "format": "date-time",
                "description": "UTC boundary before which the stated pruning affects coverage."
              },
              "reason": {
                "$ref": "#/components/schemas/CoverageReason",
                "description": "Retention or capacity policy responsible for pruning."
              },
              "sourceNodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Source Node whose retained history was pruned."
              },
              "type": {
                "type": "string",
                "enum": [
                  "pruned"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A source-reported collection gap intersects the query window.",
            "required": [
              "sourceNodeId",
              "instanceId",
              "eventId",
              "from",
              "to",
              "dropped",
              "type"
            ],
            "properties": {
              "dropped": {
                "$ref": "#/components/schemas/Sequence",
                "description": "Original number of dropped events, without estimating a query-window proportion."
              },
              "eventId": {
                "$ref": "#/components/schemas/EventId",
                "description": "Original collection-gap event identifier."
              },
              "from": {
                "type": "string",
                "format": "date-time",
                "description": "Original UTC start of the gap, without clipping to the query window."
              },
              "instanceId": {
                "$ref": "#/components/schemas/InstanceId",
                "description": "Runtime instance that reported the collection gap."
              },
              "sourceNodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Node whose collector dropped the events."
              },
              "to": {
                "type": "string",
                "format": "date-time",
                "description": "Original UTC end of the gap, without clipping to the query window."
              },
              "type": {
                "type": "string",
                "enum": [
                  "collectionGap"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Known retention boundaries and source collection gaps affecting historical coverage."
      },
      "HistoryItems": {
        "oneOf": [
          {
            "type": "object",
            "description": "Original events with authenticated source and reception metadata.",
            "required": [
              "items",
              "mode"
            ],
            "properties": {
              "items": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/HistoricalEvent"
                },
                "description": "Visible events within the bounded query page."
              },
              "mode": {
                "type": "string",
                "enum": [
                  "raw"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Aggregated real probe results without synthetic empty buckets.",
            "required": [
              "items",
              "mode"
            ],
            "properties": {
              "items": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/HistoryBucket"
                },
                "description": "Nonempty measurement buckets within the query page."
              },
              "mode": {
                "type": "string",
                "enum": [
                  "buckets"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Channel summaries derived from actual historical events.",
            "required": [
              "items",
              "mode"
            ],
            "properties": {
              "items": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/RecentLink"
                },
                "description": "Visible recent channels within the query page."
              },
              "mode": {
                "type": "string",
                "enum": [
                  "recentLinks"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Historical result projection matching the requested query mode."
      },
      "HistoryKind": {
        "type": "string",
        "description": "Historical event kind used to filter a query.",
        "enum": [
          "dial_started",
          "dial_failed",
          "link_opened",
          "link_closed",
          "probe_completed",
          "collection_gap"
        ]
      },
      "HistoryMode": {
        "type": "string",
        "description": "Projection requested from stored observation history, without initiating new measurements.",
        "enum": [
          "raw",
          "buckets",
          "recentLinks"
        ]
      },
      "HistoryPage": {
        "type": "object",
        "description": "Permission-filtered historical page with explicit coverage and a stable continuation boundary.",
        "required": [
          "from",
          "to",
          "asOf",
          "coverage",
          "result"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "UTC read upper bound retained across continuation pages."
          },
          "coverage": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/HistoryCoverage"
            },
            "description": "Known pruning and collection gaps visible under current source permissions."
          },
          "from": {
            "type": "string",
            "format": "date-time",
            "description": "UTC start of the requested history window."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Cursor for continuing this exact query, or null when no page remains."
          },
          "result": {
            "$ref": "#/components/schemas/HistoryItems",
            "description": "Historical items in the requested projection mode."
          },
          "to": {
            "type": "string",
            "format": "date-time",
            "description": "UTC end of the requested history window."
          }
        },
        "additionalProperties": false
      },
      "HistoryQuery": {
        "type": "object",
        "description": "Fixed-window history query evaluated under the caller's current permissions.",
        "required": [
          "from",
          "to",
          "mode",
          "limit"
        ],
        "properties": {
          "bucketSeconds": {
            "type": [
              "integer",
              "null"
            ],
            "format": "int32",
            "description": "Required bucket width of 1–86400 seconds in bucket mode; null or omitted in other modes.",
            "minimum": 0
          },
          "cursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Continuation cursor bound to this window, filters, mode, identity and read upper bound."
          },
          "from": {
            "type": "string",
            "format": "date-time",
            "description": "UTC start of the requested event-time window."
          },
          "kinds": {
            "type": [
              "array",
              "null"
            ],
            "items": {
              "$ref": "#/components/schemas/HistoryKind"
            },
            "description": "Optional event-kind filter; coverage reporting remains independent of this filter."
          },
          "limit": {
            "type": "integer",
            "format": "int32",
            "description": "Maximum number of result items in the page, from 1 through 200.",
            "maximum": 200,
            "minimum": 1
          },
          "linkId": {
            "type": [
              "string",
              "null"
            ],
            "description": "Optional actual channel identifier filter."
          },
          "mode": {
            "$ref": "#/components/schemas/HistoryMode",
            "description": "Raw events, probe buckets or recent channel summaries to return."
          },
          "peerNodeId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NodeId",
                "description": "Optional adjacent peer filter for relevant events."
              },
              {
                "type": "null"
              }
            ]
          },
          "sourceNodeId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NodeId",
                "description": "Optional reporting Node filter that only narrows authorized results."
              },
              {
                "type": "null"
              }
            ]
          },
          "to": {
            "type": "string",
            "format": "date-time",
            "description": "UTC end of the requested event-time window."
          }
        },
        "additionalProperties": false
      },
      "HistoryReport": {
        "type": "object",
        "description": "Batch of real historical events reported by an authenticated Node.",
        "required": [
          "instanceId",
          "events"
        ],
        "properties": {
          "events": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ObservationEvent"
            },
            "description": "Events retaining their original identifiers, occurrence times and execution contexts."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Runtime instance that produced the events."
          }
        },
        "additionalProperties": false
      },
      "InitialNodeSettings": {
        "type": "object",
        "description": "Initial configuration to validate and apply when an enrollment key registers a Node.",
        "required": [
          "listeners"
        ],
        "properties": {
          "hostname": {
            "type": [
              "string",
              "null"
            ],
            "description": "Requested hostname, or null to use the enrolling device's hostname."
          },
          "listeners": {
            "type": "object",
            "description": "Initial listeners indexed by configuration key, including supplied transport material.",
            "additionalProperties": {
              "$ref": "#/components/schemas/Listener"
            },
            "propertyNames": {
              "type": "string"
            }
          }
        },
        "additionalProperties": false
      },
      "InitialNodeSettingsView": {
        "type": "object",
        "description": "Non-secret initial settings retained with an enrollment key.",
        "required": [
          "hostname",
          "listeners"
        ],
        "properties": {
          "hostname": {
            "type": [
              "string",
              "null"
            ],
            "description": "Requested hostname, or null when the device supplies it at enrollment."
          },
          "listeners": {
            "type": "object",
            "description": "Listener summaries excluding passwords, private keys and header values.",
            "additionalProperties": {
              "$ref": "#/components/schemas/ListenerView"
            },
            "propertyNames": {
              "type": "string"
            }
          }
        },
        "additionalProperties": false
      },
      "InstanceId": {
        "type": "string",
        "format": "uuid",
        "description": "Runtime instance identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "InstanceLease": {
        "type": "object",
        "description": "Server-issued runtime qualification with separate instance and execution deadlines.",
        "required": [
          "instanceId",
          "generation",
          "expiresAt",
          "executionExpiresAt"
        ],
        "properties": {
          "executionExpiresAt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Timestamp",
                "description": "Execution deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339; null means no execution lease is granted."
              },
              {
                "type": "null"
              }
            ]
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Instance deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339."
          },
          "generation": {
            "$ref": "#/components/schemas/Version",
            "description": "Server-issued generation fencing replaced instances."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Acquired Node runtime instance."
          },
          "target": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Exact current execution target, or null when no target is available; its presence alone does not grant execution."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "InstanceRequest": {
        "type": "object",
        "description": "Exact runtime instance qualification required for an instance-bound operation.",
        "required": [
          "instanceId",
          "generation"
        ],
        "properties": {
          "generation": {
            "$ref": "#/components/schemas/Version",
            "description": "Server-issued instance generation that rejects superseded callers."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Instance identifier acquired for the current Node runtime."
          }
        },
        "additionalProperties": false
      },
      "IpInfo": {
        "type": "object",
        "description": "Independent Server-derived geographic and autonomous-system lookups for one visible IP.",
        "required": [
          "geoip",
          "asn"
        ],
        "properties": {
          "asn": {
            "$ref": "#/components/schemas/LookupResult_AsnData",
            "description": "Autonomous-system dataset result, independent of geographic lookup success."
          },
          "geoip": {
            "$ref": "#/components/schemas/LookupResult_GeoIpData",
            "description": "Geographic dataset result, including explicit absence or lookup failure."
          }
        },
        "additionalProperties": false
      },
      "IssueCode": {
        "type": "string",
        "description": "Every diagnostic a configuration check can report. Clients localize by\ncode and parameters; `message` is the English rendering of the same.",
        "enum": [
          "invalid_jsonc",
          "invalid_shape",
          "incomplete_version",
          "unknown_user",
          "invalid_group_name",
          "invalid_tag_name",
          "unknown_group",
          "unknown_tag",
          "duplicate_tag",
          "identity_membership_missing",
          "empty_tag_identity",
          "unknown_node",
          "unknown_node_selector",
          "retained_membership_inactive",
          "empty_selector_set",
          "bidirectional_requires_mesh",
          "invalid_port_set",
          "invalid_domain_suffix",
          "invalid_domain",
          "noncanonical_cidr",
          "invalid_dns_domain",
          "invalid_node_dns_name",
          "dns_override_requires_default",
          "unknown_resolver",
          "invalid_resolver",
          "empty_resolver_set",
          "duplicate_dns_suffix",
          "invalid_dns_suffix",
          "invalid_hostname",
          "duplicate_hostname",
          "empty_listener_key",
          "invalid_transport",
          "invalid_listener_materials",
          "overlapping_pools",
          "invalid_pool",
          "address_outside_pool",
          "duplicate_address",
          "unknown_reassignment_target",
          "no_address_available",
          "repeated_required_node",
          "empty_exit_set",
          "repeated_exit",
          "exit_not_enabled",
          "invalid_destination_cidr",
          "empty_route_match",
          "unknown_rule_set",
          "invalid_rule_source",
          "empty_rule_set_key",
          "empty_inline_rule_set",
          "rule_set_budget",
          "rule_set_unavailable",
          "rule_set_timeout",
          "reserved_profile_name",
          "unknown_profile",
          "default_not_allowed",
          "auto_target_not_allowed",
          "auto_target_unknown",
          "conflicting_assignments",
          "invalid_country_code",
          "exit_outside_access",
          "global_exit_requires_internet",
          "exit_fallback_requires_internet",
          "shadowed_rule"
        ]
      },
      "JoinLinkActions": {
        "type": "object",
        "description": "Current operation hints for a join link, not an authorization credential.",
        "required": [
          "revoke"
        ],
        "properties": {
          "revoke": {
            "type": "boolean",
            "description": "Whether the caller may currently revoke this link."
          }
        },
        "additionalProperties": false
      },
      "JoinLinkCreated": {
        "oneOf": [
          {
            "type": "object",
            "description": "First successful creation response carrying the one-time join credential.",
            "required": [
              "requestId",
              "link",
              "secret",
              "secretDelivery"
            ],
            "properties": {
              "link": {
                "$ref": "#/components/schemas/JoinLinkView",
                "description": "Created join link's non-secret metadata."
              },
              "requestId": {
                "$ref": "#/components/schemas/RequestId",
                "description": "Server-generated correlation ID for this HTTP request."
              },
              "secret": {
                "$ref": "#/components/schemas/WireSecret",
                "description": "Original join credential, delivered only in the first successful creation response."
              },
              "secretDelivery": {
                "type": "string",
                "enum": [
                  "available"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Idempotent creation retry whose original credential cannot be recovered.",
            "required": [
              "requestId",
              "link",
              "secretDelivery"
            ],
            "properties": {
              "link": {
                "$ref": "#/components/schemas/JoinLinkView",
                "description": "Metadata for the previously created join link."
              },
              "requestId": {
                "$ref": "#/components/schemas/RequestId",
                "description": "Server-generated correlation ID for this HTTP request."
              },
              "secretDelivery": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Join link creation result distinguishing first secret delivery from metadata-only retries."
      },
      "JoinLinkCredentialRequest": {
        "type": "object",
        "description": "Secret-bearing join link inspection or redemption input.",
        "required": [
          "token"
        ],
        "properties": {
          "token": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "One-time join credential supplied in the request body rather than the URL."
          }
        },
        "additionalProperties": false
      },
      "JoinLinkId": {
        "type": "string",
        "format": "uuid",
        "description": "Network join-link record identifier; not the joining secret.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "JoinLinkInspection": {
        "type": "object",
        "description": "Validated join link preview that does not consume the credential.",
        "required": [
          "requestId",
          "link",
          "networkName",
          "alreadyMember"
        ],
        "properties": {
          "alreadyMember": {
            "type": "boolean",
            "description": "Whether the inspecting user already has an active membership in the network."
          },
          "link": {
            "$ref": "#/components/schemas/JoinLinkView",
            "description": "Inspected link's non-secret metadata."
          },
          "networkName": {
            "type": "string",
            "description": "Current name of the network the link grants access to."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "JoinLinkListResponse": {
        "type": "object",
        "description": "A bounded page of join link metadata for an authorized network manager.",
        "required": [
          "requestId",
          "items",
          "nextCursor",
          "asOf",
          "total"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the initial pagination window was established."
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/JoinLinkView"
            },
            "description": "Join links in descending creation order within the initial query bound."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Opaque continuation cursor, or null when no further page is available."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "total": {
            "$ref": "#/components/schemas/Count",
            "description": "Total links in the authorized network within the initial creation bound."
          }
        },
        "additionalProperties": false
      },
      "JoinLinkRedeemed": {
        "type": "object",
        "description": "Membership created by redemption or recovered by a valid retry from the same consumer.",
        "required": [
          "requestId",
          "networkId",
          "membershipId"
        ],
        "properties": {
          "membershipId": {
            "$ref": "#/components/schemas/MembershipId",
            "description": "Membership created by this link, which must remain active for retry recovery."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network joined by redeeming the link."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "JoinLinkView": {
        "type": "object",
        "description": "Non-secret join link metadata with current revocation eligibility.",
        "required": [
          "id",
          "networkId",
          "role",
          "createdAt",
          "expiresAt",
          "issuedBy",
          "revokedAt",
          "consumedAt",
          "consumedBy",
          "actions"
        ],
        "properties": {
          "actions": {
            "$ref": "#/components/schemas/JoinLinkActions",
            "description": "Current caller-specific operation hints, rechecked when a mutation is committed."
          },
          "consumedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Time of successful redemption, or null while the link remains unconsumed."
          },
          "consumedBy": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/LinkUser",
                "description": "Account that redeemed the link, or null before successful redemption."
              },
              {
                "type": "null"
              }
            ]
          },
          "createdAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the link was issued."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which an unused link becomes invalid."
          },
          "id": {
            "$ref": "#/components/schemas/JoinLinkId",
            "description": "Stable identifier of this join link."
          },
          "issuedBy": {
            "$ref": "#/components/schemas/LinkUser",
            "description": "Account that issued the link under its then-current membership."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network to which redemption grants membership."
          },
          "revokedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Time of permanent revocation, or null if the link has not been revoked."
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole",
            "description": "Fixed network role granted by this link."
          }
        },
        "additionalProperties": false
      },
      "KeyAlgorithm": {
        "type": "string",
        "description": "Cryptographic algorithm of a public Node key, checked against its intended purpose.",
        "enum": [
          "ed25519",
          "x25519"
        ]
      },
      "KeyEnrollmentRequest": {
        "type": "object",
        "description": "Non-interactive registration using a one-time Tag enrollment key and device key proof.",
        "required": [
          "enrollmentKey",
          "request",
          "verifier"
        ],
        "properties": {
          "enrollmentKey": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Secret one-time enrollment credential for the target network."
          },
          "request": {
            "$ref": "#/components/schemas/CreateEnrollmentRequest",
            "description": "Signed device enrollment intent binding its own public keys."
          },
          "verifier": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "PKCE verifier matching the challenge in the signed intent."
          }
        },
        "additionalProperties": false
      },
      "KeyId": {
        "type": "string",
        "format": "uuid",
        "description": "Key record identifier; not key material.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "LinkAcquisition": {
        "oneOf": [
          {
            "type": "object",
            "description": "The channel was acquired by dialing a published endpoint.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "publishedEndpoint"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The channel was acquired by dialing an endpoint the peer Node reported from its own interfaces.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "localEndpoint"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The channel was acquired by accepting an inbound connection.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "acceptedInbound"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The UDP channel was acquired through a coordinated traversal attempt.",
            "required": [
              "attemptId",
              "endpointSource",
              "type"
            ],
            "properties": {
              "attemptId": {
                "$ref": "#/components/schemas/UdpTraversalId",
                "description": "Coordinated UDP traversal attempt that acquired the channel."
              },
              "endpointSource": {
                "$ref": "#/components/schemas/EndpointKind",
                "description": "Coarse provenance of the endpoint used for the traversal."
              },
              "type": {
                "type": "string",
                "enum": [
                  "coordinatedUdp"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "How the actual physical channel was acquired, not business authorization."
      },
      "LinkEvidence": {
        "type": "string",
        "description": "Server comparison of endpoint reports for the same physical channel.",
        "enum": [
          "oneSided",
          "matched",
          "conflicting",
          "expired"
        ]
      },
      "LinkPhase": {
        "type": "string",
        "description": "Node-reported lifecycle of an actual transport channel.",
        "enum": [
          "preparing",
          "authenticated",
          "closed",
          "failed"
        ]
      },
      "LinkState": {
        "type": "object",
        "description": "One endpoint's report of a physical channel and its current dependencies.",
        "required": [
          "linkId",
          "transport",
          "initiatorNodeId",
          "socketId",
          "phase",
          "businessDependencies",
          "observationIds",
          "observedAt",
          "expiresAt"
        ],
        "properties": {
          "acquisition": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/LinkAcquisition"
              },
              {
                "type": "null"
              }
            ]
          },
          "businessDependencies": {
            "type": "integer",
            "format": "int32",
            "description": "Number of current business dependencies, not a traffic counter or rate.",
            "minimum": 0
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC deadline after which this channel report is no longer current."
          },
          "initiatorNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that initiated establishment, independent of business traffic direction."
          },
          "linkId": {
            "type": "string",
            "description": "Non-secret channel identifier bound by the actual handshake."
          },
          "observationIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ObservationId"
            },
            "description": "Diagnostic tasks depending on this channel, separate from business dependencies."
          },
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC time at which the Node observed this channel fact."
          },
          "phase": {
            "$ref": "#/components/schemas/LinkPhase",
            "description": "Channel lifecycle stage observed by this endpoint."
          },
          "since": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Known UTC start of the reported state, or null when unknown."
          },
          "socketId": {
            "type": "string",
            "description": "Socket carrying this channel in the same report."
          },
          "transport": {
            "type": "string",
            "description": "Transport actually used by this channel."
          }
        },
        "additionalProperties": false
      },
      "LinkUser": {
        "type": "object",
        "description": "A join link's issuer or consumer, readable even after leaving the network.",
        "required": [
          "userId",
          "email"
        ],
        "properties": {
          "email": {
            "type": "string",
            "description": "Account's current email address, including after membership removal."
          },
          "userId": {
            "$ref": "#/components/schemas/UserId",
            "description": "Stable identifier of the named account."
          }
        },
        "additionalProperties": false
      },
      "Listener": {
        "oneOf": [
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "type": {
                "type": "string",
                "enum": [
                  "native_udp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "type": {
                "type": "string",
                "enum": [
                  "native_tcp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "tls",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "flow": {
                "type": "string",
                "description": "VLESS flow selection; only the empty string is accepted and omission defaults to it."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "stream": {
                "$ref": "#/components/schemas/VlessStream",
                "description": "VLESS encapsulation, `tcp` or `websocket`, both over TLS; defaults to `tcp`."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "tls": {
                "$ref": "#/components/schemas/ListenerTls",
                "description": "Required TLS server settings, including the certificate verification name and material source."
              },
              "type": {
                "type": "string",
                "enum": [
                  "vless"
                ]
              },
              "uuid": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Hyphenated UUID credential with case-insensitive hexadecimal digits; omission makes the Server generate or retain saved material, while null is rejected."
              },
              "websocket": {
                "$ref": "#/components/schemas/ListenerWebSocket",
                "description": "WebSocket Upgrade settings; omission uses defaults for a websocket stream, while a tcp stream forbids this field and null is rejected."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "tls",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "password": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Nonempty password preserved without trimming or case normalization; omission makes the Server generate or retain saved material, while null is rejected."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "stream": {
                "$ref": "#/components/schemas/TrojanStream",
                "description": "Trojan encapsulation; only `tcp` is accepted and omission defaults to it."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "tls": {
                "$ref": "#/components/schemas/ListenerTls",
                "description": "Required TLS server settings, including the certificate verification name and material source."
              },
              "type": {
                "type": "string",
                "enum": [
                  "trojan"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "An inbound listener as declared: `type` selects the carrier; credential and TLS materials may be omitted for the Server to generate."
      },
      "ListenerChange": {
        "type": "object",
        "description": "One listener affected by a proposed configuration change.",
        "required": [
          "nodeId",
          "key",
          "kind"
        ],
        "properties": {
          "key": {
            "type": "string",
            "description": "Node-local listener key used to locate the resource."
          },
          "kind": {
            "$ref": "#/components/schemas/ChangeKind",
            "description": "Whether the listener is added, removed or changed, including planned material generation."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that owns the changed listener."
          }
        },
        "additionalProperties": false
      },
      "ListenerDelta": {
        "type": "object",
        "description": "Server-computed keyed changes between two complete execution targets.",
        "required": [
          "remove",
          "upsert"
        ],
        "properties": {
          "remove": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Keys present in the base that must be removed from the reconstructed target."
          },
          "upsert": {
            "type": "object",
            "description": "Complete values to insert or replace; keys cannot also occur in remove.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PreparedListener"
            },
            "propertyNames": {
              "type": "string"
            }
          }
        },
        "additionalProperties": false
      },
      "ListenerPhase": {
        "type": "string",
        "description": "Local listener lifecycle reported by the Node.",
        "enum": [
          "preparing",
          "listening",
          "stopped",
          "failed"
        ]
      },
      "ListenerState": {
        "type": "object",
        "description": "Runtime facts for a configured listener key.",
        "required": [
          "phase",
          "socketIds"
        ],
        "properties": {
          "configuration": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Explicit execution target; null or omission inherits the report's applied configuration."
              },
              {
                "type": "null"
              }
            ]
          },
          "loadedCertificateSha256": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Digest",
                "description": "Public fingerprint of the certificate actually loaded, or null for non-TLS or unavailable results."
              },
              {
                "type": "null"
              }
            ]
          },
          "phase": {
            "$ref": "#/components/schemas/ListenerPhase",
            "description": "Local lifecycle stage observed by the Node."
          },
          "problem": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/StateProblem",
                "description": "Typed listener failure, or null when none is reported."
              },
              {
                "type": "null"
              }
            ]
          },
          "socketIds": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Actual socket IDs in the same Node report."
          }
        },
        "additionalProperties": false
      },
      "ListenerTls": {
        "type": "object",
        "description": "TLS server parameters and certificate material for a VLESS or Trojan listener.",
        "required": [
          "serverName"
        ],
        "properties": {
          "alpn": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Ordered ALPN names of 1–255 bytes without control characters; defaults to empty, and WebSocket permits only `http/1.1` entries."
          },
          "certificate": {
            "$ref": "#/components/schemas/SecretString",
            "description": "Nonempty PEM certificate chain, leaf first; required in provided mode, otherwise omission generates or retains saved material and null is rejected."
          },
          "handshakeTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive TLS handshake budget; defaults to `5s` and cannot extend the overall connection or remaining lease budget."
          },
          "maxVersion": {
            "$ref": "#/components/schemas/TlsVersion",
            "description": "Maximum accepted TLS version, `1.2` or `1.3`; defaults to `1.3` and must not precede minVersion."
          },
          "minVersion": {
            "$ref": "#/components/schemas/TlsVersion",
            "description": "Minimum accepted TLS version, `1.2` or `1.3`; defaults to `1.2` and must not exceed maxVersion."
          },
          "mode": {
            "$ref": "#/components/schemas/TlsMode",
            "description": "Certificate-material source; defaults to `managed`, while `provided` requires both certificate and private key."
          },
          "privateKey": {
            "$ref": "#/components/schemas/SecretString",
            "description": "Nonempty matching PEM private key; required in provided mode, otherwise omission generates or retains saved material and null is rejected."
          },
          "serverName": {
            "type": "string",
            "description": "Required TLS host name or IP address used for certificate identity and the dialer's verification name."
          }
        },
        "additionalProperties": false
      },
      "ListenerTlsView": {
        "type": "object",
        "description": "TLS listener settings with material presence indicators instead of certificate and key contents.",
        "required": [
          "mode",
          "serverName",
          "minVersion",
          "maxVersion",
          "alpn",
          "handshakeTimeout",
          "certificatePresent",
          "privateKeyPresent"
        ],
        "properties": {
          "alpn": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Ordered application protocols offered for negotiation; WebSocket requires HTTP/1.1-compatible values."
          },
          "certificatePresent": {
            "type": "boolean",
            "description": "Whether certificate material is present in this declaration; preview generation plans may leave it false."
          },
          "handshakeTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive TLS handshake budget, also bounded by the remaining connection and execution deadlines."
          },
          "maxVersion": {
            "$ref": "#/components/schemas/TlsVersion",
            "description": "Highest permitted TLS version, no lower than `minVersion`."
          },
          "minVersion": {
            "$ref": "#/components/schemas/TlsVersion",
            "description": "Lowest permitted TLS version, no greater than `maxVersion`."
          },
          "mode": {
            "$ref": "#/components/schemas/TlsMode",
            "description": "Whether certificate materials are managed by the platform or provided by the administrator."
          },
          "privateKeyPresent": {
            "type": "boolean",
            "description": "Whether private-key material is present in this declaration; preview generation plans may leave it false."
          },
          "serverName": {
            "type": "string",
            "description": "TLS name used for certificate verification and peer dialing."
          }
        },
        "additionalProperties": false
      },
      "ListenerUpdate": {
        "oneOf": [
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "type": {
                "type": "string",
                "enum": [
                  "native_udp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "type": {
                "type": "string",
                "enum": [
                  "native_tcp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "VLESS listener replacement with retention of omitted materials within the same resource.",
            "required": [
              "listenPort",
              "tls",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Requested underlay interface; defaults to an empty string for platform selection."
              },
              "enabled": {
                "type": "boolean",
                "description": "Request creation of the fixed listener; defaults to true."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external host/port pairs; defaults to an empty list."
              },
              "flow": {
                "type": "string",
                "description": "VLESS flow; only the default empty string is supported."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0`."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Free-form description; defaults to an empty string."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests in bytes; omitted settings use zero for provider defaults."
              },
              "stream": {
                "$ref": "#/components/schemas/VlessStream",
                "description": "VLESS encapsulation, `tcp` or `websocket`; defaults to `tcp` and both use TLS over TCP."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "TCP no-delay and keepalive settings; omission uses the shared TCP defaults."
              },
              "tls": {
                "$ref": "#/components/schemas/ListenerTls",
                "description": "TLS declaration; omitted materials are retained for the same mode, and managed certificates are regenerated when the server name changes."
              },
              "type": {
                "type": "string",
                "enum": [
                  "vless"
                ]
              },
              "uuid": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Protocol UUID; omission retains this resource's saved UUID or generates one initially, while null is rejected."
              },
              "websocket": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/ListenerWebSocketUpdate",
                    "description": "Omission retains this resource's WebSocket options, null removes them, and a value updates them; only allowed with `websocket` stream."
                  },
                  {
                    "type": "null"
                  }
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "tls",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "password": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Nonempty password preserved without trimming or case normalization; omission makes the Server generate or retain saved material, while null is rejected."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "stream": {
                "$ref": "#/components/schemas/TrojanStream",
                "description": "Trojan encapsulation; only `tcp` is accepted and omission defaults to it."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "tls": {
                "$ref": "#/components/schemas/ListenerTls",
                "description": "Required TLS server settings, including the certificate verification name and material source."
              },
              "type": {
                "type": "string",
                "enum": [
                  "trojan"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "A listener edit keyed by the listener's name: `type` selects the carrier; omitted VLESS secrets, TLS materials, WebSocket block and response headers keep the saved values of the same listener."
      },
      "ListenerView": {
        "type": "object",
        "description": "Read-only listener declaration with credentials and sensitive header values removed.",
        "required": [
          "protocol",
          "enabled",
          "remark",
          "listen",
          "listenPort",
          "endpoints",
          "bindInterface",
          "socket",
          "credentialPresent"
        ],
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Requested underlay interface; an empty string leaves interface selection to the platform."
          },
          "credentialPresent": {
            "type": "boolean",
            "description": "Whether a VLESS UUID or Trojan password is present; false for Native transports and unprepared automatic credentials."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether the configuration requests creation of this listener, rather than its actual installation state."
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/Endpoint"
            },
            "description": "Explicitly advertised external host/port pairs; their presence does not establish reachability."
          },
          "flow": {
            "type": [
              "string",
              "null"
            ],
            "description": "VLESS flow, currently always empty when present; null for other protocols."
          },
          "listen": {
            "type": "string",
            "description": "Local IPv4 or IPv6 bind address, distinct from advertised endpoints."
          },
          "listenPort": {
            "type": "integer",
            "format": "int32",
            "description": "Local listening port in 1–65535.",
            "minimum": 0
          },
          "protocol": {
            "$ref": "#/components/schemas/TransportProtocol",
            "description": "Outer transport protocol configured for this fixed inbound resource."
          },
          "remark": {
            "type": "string",
            "description": "Free-form description with no authorization meaning."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Receive and send buffer requests in bytes, with zero selecting provider defaults."
          },
          "stream": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/VlessStream",
                "description": "VLESS stream encapsulation; null for other protocols."
              },
              {
                "type": "null"
              }
            ]
          },
          "tcp": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "TCP socket options; null for UDP listeners."
              },
              {
                "type": "null"
              }
            ]
          },
          "tls": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ListenerTlsView",
                "description": "Redacted TLS settings for VLESS or Trojan; null for Native transports."
              },
              {
                "type": "null"
              }
            ]
          },
          "websocket": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ListenerWebSocketView",
                "description": "Redacted WebSocket settings when configured for VLESS; null otherwise."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "ListenerWebSocket": {
        "type": "object",
        "description": "HTTP/1.1 WebSocket Upgrade parameters supplied by the target VLESS listener.",
        "properties": {
          "host": {
            "type": "string",
            "description": "Exact WebSocket Host name or IP address; omission uses TLS serverName, while null is rejected."
          },
          "path": {
            "$ref": "#/components/schemas/WebSocketPath",
            "description": "Exact path beginning with `/`, without query, fragment, whitespace, or control characters; defaults to `/meshway`."
          },
          "responseHeaders": {
            "$ref": "#/components/schemas/WebSocketHeaders",
            "description": "Additional response headers, excluding reserved handshake and framing names; defaults to an empty object."
          }
        },
        "additionalProperties": false
      },
      "ListenerWebSocketUpdate": {
        "type": "object",
        "description": "Replacement WebSocket listener options with retention of omitted response headers.",
        "properties": {
          "host": {
            "type": "string",
            "description": "Explicit HTTP Host; omission uses the TLS server name and null is rejected."
          },
          "path": {
            "$ref": "#/components/schemas/WebSocketPath",
            "description": "Exact request path beginning with `/`, without query, fragment or whitespace; defaults to `/meshway`."
          },
          "responseHeaders": {
            "$ref": "#/components/schemas/WebSocketHeaders",
            "description": "Omission retains extra response headers, `{}` clears them, and a nonempty map replaces them without overriding handshake or framing headers."
          }
        },
        "additionalProperties": false
      },
      "ListenerWebSocketView": {
        "type": "object",
        "description": "Public WebSocket listener settings without response header values.",
        "required": [
          "path",
          "responseHeaderNames"
        ],
        "properties": {
          "host": {
            "type": [
              "string",
              "null"
            ],
            "description": "Explicit HTTP Host override; null uses the listener TLS server name."
          },
          "path": {
            "$ref": "#/components/schemas/WebSocketPath",
            "description": "Exact request path beginning with `/`, without query, fragment, spaces or control characters."
          },
          "responseHeaderNames": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Names of extra response headers, with all values withheld."
          }
        },
        "additionalProperties": false
      },
      "LocalEgressAction": {
        "type": "string",
        "description": "Local egress selection bound into a source-only execution permit.",
        "enum": [
          "direct",
          "unmanaged"
        ]
      },
      "LocalEgressDelta": {
        "type": "object",
        "description": "Server-computed keyed changes between two complete execution targets.",
        "required": [
          "remove",
          "upsert"
        ],
        "properties": {
          "remove": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/RequestId"
            },
            "description": "Keys present in the base that must be removed from the reconstructed target."
          },
          "upsert": {
            "type": "object",
            "description": "Complete values to insert or replace; keys cannot also occur in remove.",
            "additionalProperties": {
              "$ref": "#/components/schemas/LocalEgressPermit"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "LocalEgressPermit": {
        "type": "object",
        "description": "Server-compiled exact local egress binding published only to the business source.",
        "required": [
          "demandId",
          "source",
          "target",
          "protocol",
          "profile",
          "action",
          "expiresAt"
        ],
        "properties": {
          "action": {
            "$ref": "#/components/schemas/LocalEgressAction",
            "description": "Selected local routing action, without Mesh relay or exit admission."
          },
          "demandId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Demand authorizing this local execution binding."
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Local permit deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and bounded by its dependencies."
          },
          "port": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Port",
                "description": "Exact TCP or UDP destination port; null or omitted only for ICMP echo."
              },
              {
                "type": "null"
              }
            ]
          },
          "profile": {
            "type": "string",
            "description": "Routing profile under which the local action was selected."
          },
          "protocol": {
            "$ref": "#/components/schemas/BusinessProtocol",
            "description": "Authorized inner business protocol."
          },
          "source": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Only Node allowed to consume the local binding."
          },
          "target": {
            "$ref": "#/components/schemas/BusinessTarget",
            "description": "Exact destination covered by the local route."
          }
        },
        "additionalProperties": false
      },
      "LocalEndpoint": {
        "type": "object",
        "description": "Dialable address the Node reported for a listener socket, derived from its own interfaces.",
        "required": [
          "address",
          "source"
        ],
        "properties": {
          "address": {
            "type": "string",
            "description": "Interface address and the listener's bound port."
          },
          "source": {
            "$ref": "#/components/schemas/LocalEndpointSource",
            "description": "Where the address came from."
          }
        },
        "additionalProperties": false
      },
      "LocalEndpointSource": {
        "type": "string",
        "description": "Provenance of an endpoint the Node reported for one of its own listeners.",
        "enum": [
          "local"
        ]
      },
      "LocationAbsentReason": {
        "type": "string",
        "description": "Reason the Server cannot derive a current approximate Node location.",
        "enum": [
          "noObservation",
          "observationExpired",
          "datasetUnavailable",
          "notInDataset",
          "lookupFailed",
          "ambiguousObservation"
        ]
      },
      "LocationSource": {
        "type": "string",
        "description": "Eligible local endpoint observation used to derive approximate Node location.",
        "enum": [
          "interface",
          "stun"
        ]
      },
      "LogicalPath": {
        "type": "object",
        "description": "Complete ordered path authorized by Server compilation, including all required intermediate Nodes.",
        "required": [
          "nodes",
          "dialCandidates"
        ],
        "properties": {
          "dialCandidates": {
            "type": "object",
            "description": "Candidate keys grouped by the actual dial initiator, independent of business direction.",
            "additionalProperties": {
              "type": "array",
              "items": {
                "type": "string"
              }
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Ordered source, intermediate Nodes and terminal Node or exit."
          }
        },
        "additionalProperties": false
      },
      "LoginRequest": {
        "type": "object",
        "description": "Credentials for issuing a user session for an existing account.",
        "required": [
          "email",
          "password"
        ],
        "properties": {
          "email": {
            "type": "string",
            "description": "Login address normalized using the same rules as registration."
          },
          "password": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Account password, compared without trimming or rewriting."
          }
        },
        "additionalProperties": false
      },
      "LookupResult_AsnData": {
        "oneOf": [
          {
            "type": "object",
            "description": "The selected dataset contains a valid result for the address.",
            "required": [
              "datasetId",
              "lookedUpAt",
              "data",
              "status"
            ],
            "properties": {
              "data": {
                "type": "object",
                "description": "Dataset mapping from an IP address to its autonomous system, not a live routing measurement.",
                "required": [
                  "number"
                ],
                "properties": {
                  "number": {
                    "type": "integer",
                    "format": "int32",
                    "description": "Actual autonomous system number; zero is not an unknown-value placeholder.",
                    "minimum": 0
                  },
                  "organization": {
                    "type": [
                      "string",
                      "null"
                    ],
                    "description": "Dataset-recorded network organization, or null if absent; this does not identify the Node owner."
                  },
                  "prefix": {
                    "type": [
                      "string",
                      "null"
                    ],
                    "description": "Matching IP prefix returned by the dataset, or null if absent."
                  }
                },
                "additionalProperties": false
              },
              "datasetId": {
                "$ref": "#/components/schemas/ArtifactId",
                "description": "Immutable dataset artifact used for the lookup."
              },
              "lookedUpAt": {
                "type": "string",
                "format": "date-time",
                "description": "UTC time of the actual lookup, retained when its result is cached."
              },
              "status": {
                "type": "string",
                "enum": [
                  "available"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The lookup succeeded but the selected dataset has no entry for the address.",
            "required": [
              "datasetId",
              "lookedUpAt",
              "status"
            ],
            "properties": {
              "datasetId": {
                "$ref": "#/components/schemas/ArtifactId",
                "description": "Immutable dataset artifact searched for the address."
              },
              "lookedUpAt": {
                "type": "string",
                "format": "date-time",
                "description": "UTC time of the actual lookup, retained when its result is cached."
              },
              "status": {
                "type": "string",
                "enum": [
                  "notFound"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "No valid lookup result could be obtained.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/LookupUnavailableReason",
                "description": "Dataset availability or lookup failure preventing a result."
              },
              "status": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The address is ineligible for public IP metadata lookup.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/NotApplicableReason",
                "description": "Address classification that excludes the lookup."
              },
              "status": {
                "type": "string",
                "enum": [
                  "notApplicable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Server IP metadata lookup result retaining dataset identity and original query time."
      },
      "LookupResult_GeoIpData": {
        "oneOf": [
          {
            "type": "object",
            "description": "The selected dataset contains a valid result for the address.",
            "required": [
              "datasetId",
              "lookedUpAt",
              "data",
              "status"
            ],
            "properties": {
              "data": {
                "type": "object",
                "description": "Approximate geographic metadata returned by the Server's IP dataset.",
                "properties": {
                  "city": {
                    "type": [
                      "string",
                      "null"
                    ],
                    "description": "Dataset-provided city name, or null if absent."
                  },
                  "country": {
                    "type": [
                      "string",
                      "null"
                    ],
                    "description": "Dataset-provided ISO 3166-1 alpha-2 country or territory code, or null if absent."
                  },
                  "position": {
                    "oneOf": [
                      {
                        "$ref": "#/components/schemas/GeoPosition",
                        "description": "Approximate coordinates supplied by the dataset, or null when none are available."
                      },
                      {
                        "type": "null"
                      }
                    ]
                  },
                  "region": {
                    "type": [
                      "string",
                      "null"
                    ],
                    "description": "Dataset-provided first-level region name or identifier, or null if absent."
                  }
                },
                "additionalProperties": false
              },
              "datasetId": {
                "$ref": "#/components/schemas/ArtifactId",
                "description": "Immutable dataset artifact used for the lookup."
              },
              "lookedUpAt": {
                "type": "string",
                "format": "date-time",
                "description": "UTC time of the actual lookup, retained when its result is cached."
              },
              "status": {
                "type": "string",
                "enum": [
                  "available"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The lookup succeeded but the selected dataset has no entry for the address.",
            "required": [
              "datasetId",
              "lookedUpAt",
              "status"
            ],
            "properties": {
              "datasetId": {
                "$ref": "#/components/schemas/ArtifactId",
                "description": "Immutable dataset artifact searched for the address."
              },
              "lookedUpAt": {
                "type": "string",
                "format": "date-time",
                "description": "UTC time of the actual lookup, retained when its result is cached."
              },
              "status": {
                "type": "string",
                "enum": [
                  "notFound"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "No valid lookup result could be obtained.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/LookupUnavailableReason",
                "description": "Dataset availability or lookup failure preventing a result."
              },
              "status": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The address is ineligible for public IP metadata lookup.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/NotApplicableReason",
                "description": "Address classification that excludes the lookup."
              },
              "status": {
                "type": "string",
                "enum": [
                  "notApplicable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Server IP metadata lookup result retaining dataset identity and original query time."
      },
      "LookupUnavailableReason": {
        "type": "string",
        "description": "Reason the Server could not perform an IP metadata lookup.",
        "enum": [
          "datasetUnavailable",
          "lookupFailed"
        ]
      },
      "MaterialRequirement": {
        "type": "string",
        "enum": [
          "uuid",
          "password",
          "certificate",
          "privateKey",
          "certificateValidation"
        ]
      },
      "MemberActions": {
        "type": "object",
        "description": "Target-specific membership operation hints, including last-Owner protection.",
        "required": [
          "changeRole",
          "assignableRoles",
          "remove"
        ],
        "properties": {
          "assignableRoles": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MembershipRole"
            },
            "description": "Roles currently assignable to this member, ordered owner, admin, then member."
          },
          "changeRole": {
            "type": "boolean",
            "description": "Whether the caller may assign a role different from the member's current role."
          },
          "remove": {
            "type": "boolean",
            "description": "Whether the caller may remove this member or leave their own membership."
          }
        },
        "additionalProperties": false
      },
      "MemberMutationResponse": {
        "type": "object",
        "description": "Updated membership and current caller-specific action hints.",
        "required": [
          "requestId",
          "member"
        ],
        "properties": {
          "member": {
            "$ref": "#/components/schemas/MemberSummary",
            "description": "Membership metadata after the mutation."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "MemberRemovedResponse": {
        "type": "object",
        "description": "Committed membership removal result.",
        "required": [
          "requestId",
          "revision"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Resulting saved configuration revision after membership cleanup."
          }
        },
        "additionalProperties": false
      },
      "MemberSummary": {
        "type": "object",
        "description": "Current network membership with caller-specific operation hints.",
        "required": [
          "membershipId",
          "userId",
          "email",
          "role",
          "version",
          "joinedAt",
          "actions"
        ],
        "properties": {
          "actions": {
            "$ref": "#/components/schemas/MemberActions",
            "description": "Operations the caller may currently perform on this member."
          },
          "email": {
            "type": "string",
            "description": "Member's current canonical login address."
          },
          "joinedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which this membership was created."
          },
          "membershipId": {
            "$ref": "#/components/schemas/MembershipId",
            "description": "Stable identifier of this membership, not reused when a removed user rejoins."
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole",
            "description": "Member's current role within this network."
          },
          "userId": {
            "$ref": "#/components/schemas/UserId",
            "description": "Stable account identifier of the member."
          },
          "version": {
            "$ref": "#/components/schemas/Version",
            "description": "Concurrency version required by membership mutations."
          }
        },
        "additionalProperties": false
      },
      "MembershipId": {
        "type": "string",
        "format": "uuid",
        "description": "Network membership identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "MembershipRole": {
        "type": "string",
        "description": "A user's role within one network; this does not grant instance privileges.",
        "enum": [
          "owner",
          "admin",
          "member"
        ]
      },
      "MeshRoute": {
        "type": "object",
        "description": "First-match routing requirement recursively evaluated for each segment's actual endpoints.",
        "required": [
          "src",
          "dst",
          "via"
        ],
        "properties": {
          "dst": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Nonempty destination Mesh selectors using the same syntax as `src`."
          },
          "remark": {
            "type": "string",
            "description": "Free-form description, defaulting to an empty string; it is not a stable rule identifier."
          },
          "src": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Nonempty source selectors: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`."
          },
          "via": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Ordered required `node:<id>` waypoints; segment boundary occurrences are satisfied and actual repeats or cycles are rejected."
          }
        },
        "additionalProperties": false
      },
      "MetricKind": {
        "type": "string",
        "description": "Requested observation metric, whose availability is reported separately.",
        "enum": [
          "rtt",
          "traffic",
          "probeLoss"
        ]
      },
      "NativeTcpDialer": {
        "type": "object",
        "description": "Outbound carrier selection and connection options; the Server resolves target listener parameters separately.",
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "connectTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive total connection budget including resolution and required handshakes; defaults to `5s` and is limited by the remaining active lease."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether this dialer may be used for outgoing connections; defaults to true."
          },
          "localAddress": {
            "type": "string",
            "description": "Local outgoing IP address; omitted or empty selects automatically, and a Mesh address cannot bypass underlay protection."
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "description": "Preference from 0–65535, lower first; defaults to 100 for native_udp, 200 for native_tcp, 300 for vless, or 400 for trojan.",
            "minimum": 0
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text rather than a resource key; defaults to an empty string."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
          }
        },
        "additionalProperties": false
      },
      "NativeTcpListener": {
        "type": "object",
        "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
        "required": [
          "listenPort"
        ],
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/Endpoint"
            },
            "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
          },
          "listen": {
            "type": "string",
            "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
          },
          "listenPort": {
            "$ref": "#/components/schemas/Port",
            "description": "Required local listening port in the range 1–65535."
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text; defaults to an empty string and cannot be null."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
          }
        },
        "additionalProperties": false
      },
      "NativeUdpDialer": {
        "type": "object",
        "description": "Outbound carrier selection and connection options; the Server resolves target listener parameters separately.",
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "connectTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive total connection budget including resolution and required handshakes; defaults to `5s` and is limited by the remaining active lease."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether this dialer may be used for outgoing connections; defaults to true."
          },
          "localAddress": {
            "type": "string",
            "description": "Local outgoing IP address; omitted or empty selects automatically, and a Mesh address cannot bypass underlay protection."
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "description": "Preference from 0–65535, lower first; defaults to 100 for native_udp, 200 for native_tcp, 300 for vless, or 400 for trojan.",
            "minimum": 0
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text rather than a resource key; defaults to an empty string."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          }
        },
        "additionalProperties": false
      },
      "NativeUdpListener": {
        "type": "object",
        "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
        "required": [
          "listenPort"
        ],
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/Endpoint"
            },
            "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
          },
          "listen": {
            "type": "string",
            "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
          },
          "listenPort": {
            "$ref": "#/components/schemas/Port",
            "description": "Required local listening port in the range 1–65535."
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text; defaults to an empty string and cannot be null."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          }
        },
        "additionalProperties": false
      },
      "NetworkActions": {
        "type": "object",
        "description": "Current network operation hints; mutations independently recheck authorization.",
        "required": [
          "rename",
          "readConfiguration",
          "editConfiguration",
          "manageMembers",
          "joinLinkRoles"
        ],
        "properties": {
          "editConfiguration": {
            "type": "boolean",
            "description": "Whether the caller may edit the complete network configuration."
          },
          "joinLinkRoles": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MembershipRole"
            },
            "description": "Roles the caller may grant through join links, ordered owner, admin, then member."
          },
          "manageMembers": {
            "type": "boolean",
            "description": "Whether the caller may read and manage network members, subject to target-role constraints."
          },
          "readConfiguration": {
            "type": "boolean",
            "description": "Whether `GET /configuration` (the complete, management view) is\nreadable; otherwise the member view of own devices applies."
          },
          "rename": {
            "type": "boolean",
            "description": "Whether the caller may rename this network."
          }
        },
        "additionalProperties": false
      },
      "NetworkConfiguration": {
        "type": "object",
        "description": "Complete canonical network configuration with fixed defaults, addresses and prepared materials.",
        "required": [
          "addressPools",
          "dns"
        ],
        "properties": {
          "access": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/AccessRule"
            },
            "description": "Additive traffic grants; defaults to an empty list, denying new Mesh business traffic."
          },
          "addressPools": {
            "$ref": "#/components/schemas/AddressPools",
            "description": "Network-wide usable Mesh address pools."
          },
          "defaults": {
            "$ref": "#/components/schemas/Defaults",
            "description": "Unmatched-rule behavior and inherited dialers; omission supplies platform defaults."
          },
          "dns": {
            "$ref": "#/components/schemas/DnsConfiguration",
            "description": "Network naming and explicit resolver routing."
          },
          "groups": {
            "type": "object",
            "description": "`group:<name>` keys mapped to direct member emails, without nested groups; defaults to an empty map.",
            "additionalProperties": {
              "type": "array",
              "items": {
                "type": "string"
              }
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "nodes": {
            "type": "object",
            "description": "Registered node IDs mapped to canonical settings; adding a key does not enroll a node.",
            "additionalProperties": {
              "$ref": "#/components/schemas/NodeSettings"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "routing": {
            "$ref": "#/components/schemas/RoutingConfiguration",
            "description": "Mesh path constraints and egress selection; defaults to empty routing declarations."
          },
          "ruleSets": {
            "type": "object",
            "description": "Named inline or remote destination sets referenced by routing rules; defaults to an empty map.",
            "additionalProperties": {
              "$ref": "#/components/schemas/RuleSet"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "tagsOwners": {
            "type": "object",
            "description": "`tag:<name>` keys mapped to owner emails or `group:<name>` references; empty owner lists restrict assignment to network administrators.",
            "additionalProperties": {
              "type": "array",
              "items": {
                "type": "string"
              }
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "topology": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TopologyRule"
            },
            "description": "Undirected adjacency rules; defaults to an empty list governed by `defaults.topology`."
          }
        },
        "additionalProperties": false
      },
      "NetworkId": {
        "type": "string",
        "format": "uuid",
        "description": "Isolated network identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "NetworkListResponse": {
        "type": "object",
        "description": "A bounded page of networks in which the caller currently has membership.",
        "required": [
          "requestId",
          "items",
          "nextCursor",
          "asOf",
          "total"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the initial pagination window was established."
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NetworkSummary"
            },
            "description": "Visible networks in ascending creation order within the initial query bounds."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Opaque continuation cursor, or null when no further page is available."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "total": {
            "$ref": "#/components/schemas/Count",
            "description": "Current visible count within the cursor's original creation bounds.\nRecomputed on every page; permission loss can reduce this value."
          }
        },
        "additionalProperties": false
      },
      "NetworkMembersResponse": {
        "type": "object",
        "description": "A bounded page of current network members for an authorized manager.",
        "required": [
          "requestId",
          "items",
          "nextCursor",
          "asOf",
          "total"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the initial pagination window was established."
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MemberSummary"
            },
            "description": "Current members in ascending membership creation order."
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ],
            "description": "Opaque continuation cursor, or null when no further page is available."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "total": {
            "$ref": "#/components/schemas/Count",
            "description": "Current visible count within the cursor's original creation bound.\nRemoved members disappear and reduce total on subsequent pages."
          }
        },
        "additionalProperties": false
      },
      "NetworkNameRules": {
        "type": "object",
        "description": "Validation and normalization rules for network creation and rename input.",
        "required": [
          "minCodepoints",
          "maxCodepoints",
          "maxInputUtf8Bytes",
          "trim",
          "forbidControlCharacters"
        ],
        "properties": {
          "forbidControlCharacters": {
            "type": "boolean",
            "description": "Reject every Unicode General_Category=Cc character in raw input."
          },
          "maxCodepoints": {
            "type": "integer",
            "format": "int32",
            "description": "Maximum number of Unicode scalar values after trimming.",
            "maximum": 65535,
            "minimum": 0
          },
          "maxInputUtf8Bytes": {
            "type": "integer",
            "format": "int32",
            "description": "Maximum UTF-8 byte length of the raw input before trimming.",
            "maximum": 65535,
            "minimum": 0
          },
          "minCodepoints": {
            "type": "integer",
            "format": "int32",
            "description": "Minimum number of Unicode scalar values after trimming.",
            "maximum": 65535,
            "minimum": 0
          },
          "trim": {
            "$ref": "#/components/schemas/NetworkNameTrim",
            "description": "Unicode White_Space: U+0009..000D, 0020, 0085, 00A0, 1680,\n2000..200A, 2028, 2029, 202F, 205F, 3000. U+FEFF is not trimmed.\nRaw input Cc rejection happens before trimming (including edge controls)."
          }
        },
        "additionalProperties": false
      },
      "NetworkNameTrim": {
        "type": "string",
        "description": "Whitespace normalization applied to network names after raw input validation.",
        "enum": [
          "unicodeWhiteSpace"
        ]
      },
      "NetworkResponse": {
        "type": "object",
        "description": "A network projected for the caller's current membership.",
        "required": [
          "requestId",
          "network"
        ],
        "properties": {
          "network": {
            "$ref": "#/components/schemas/NetworkSummary",
            "description": "Network metadata and the caller's current role and action hints."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "NetworkSummary": {
        "type": "object",
        "description": "Network metadata and the requesting user's current membership and permissions.",
        "required": [
          "id",
          "name",
          "createdAt",
          "currentRevision",
          "membershipId",
          "role",
          "actions"
        ],
        "properties": {
          "actions": {
            "$ref": "#/components/schemas/NetworkActions",
            "description": "Operations currently available to the requesting user."
          },
          "createdAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the network was created."
          },
          "currentRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Current saved configuration revision, independent of Node application status."
          },
          "id": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Stable identifier of the network."
          },
          "membershipId": {
            "$ref": "#/components/schemas/MembershipId",
            "description": "Requesting user's current membership in this network."
          },
          "name": {
            "type": "string",
            "description": "Validated network name after whitespace normalization."
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole",
            "description": "Requesting user's current network role."
          }
        },
        "additionalProperties": false
      },
      "NodeAddresses": {
        "type": "object",
        "description": "Desired Mesh addresses, unique within the network and inside its usable pools.",
        "required": [
          "ipv4",
          "ipv6"
        ],
        "properties": {
          "ipv4": {
            "type": [
              "string",
              "null"
            ],
            "format": "ipv4",
            "description": "Desired Mesh IPv4 address; null means this node has no IPv4 assignment."
          },
          "ipv6": {
            "type": [
              "string",
              "null"
            ],
            "format": "ipv6",
            "description": "Desired Mesh IPv6 address; null means this node has no IPv6 assignment."
          }
        },
        "additionalProperties": false
      },
      "NodeAddressesInput": {
        "type": "object",
        "description": "Per-family address edits with explicit retention, allocation and clearing semantics.",
        "properties": {
          "ipv4": {
            "type": [
              "string",
              "null"
            ],
            "description": "Omission retains an existing IPv4 assignment or allocates initially; null clears it, and an address must be unique and in a usable pool."
          },
          "ipv6": {
            "type": [
              "string",
              "null"
            ],
            "description": "Omission retains an existing IPv6 assignment or allocates initially; null clears it, and an address must be unique and in a usable pool."
          }
        },
        "additionalProperties": false
      },
      "NodeChallenge": {
        "type": "object",
        "description": "Server-issued one-time challenge with exact identity, operation and deadline bindings.",
        "required": [
          "id",
          "controllerId",
          "networkId",
          "nodeId",
          "keyId",
          "keyGeneration",
          "nonce",
          "operation",
          "requestDigest",
          "expiresAt"
        ],
        "properties": {
          "controllerId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Durable Server installation identifier pinned alongside its HTTPS origin."
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC deadline after which the challenge cannot be consumed for a new operation."
          },
          "id": {
            "$ref": "#/components/schemas/ChallengeId",
            "description": "Client-chosen challenge application identifier retained by the Server."
          },
          "keyGeneration": {
            "$ref": "#/components/schemas/Version",
            "description": "Bound key generation checked when the proof is consumed."
          },
          "keyId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Current control key whose signature consumes the challenge."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network bound into the proof transcript."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Registered Node bound into the proof transcript."
          },
          "nonce": {
            "$ref": "#/components/schemas/Digest",
            "description": "Server-generated 32-byte random challenge nonce."
          },
          "operation": {
            "$ref": "#/components/schemas/ProofOperation",
            "description": "Only operation for which the challenge is valid."
          },
          "requestDigest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Digest of the exact typed intent authorized by this challenge."
          }
        },
        "additionalProperties": false
      },
      "NodeChallengeIntent": {
        "type": "object",
        "description": "Node-signed challenge application binding the pinned Server, registered identity and requested operation.",
        "required": [
          "id",
          "controllerId",
          "request",
          "expiresAt"
        ],
        "properties": {
          "controllerId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Durable Server installation identity pinned alongside the HTTPS origin at enrollment."
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Requested deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and limited to 60 seconds after Server admission."
          },
          "id": {
            "$ref": "#/components/schemas/ChallengeId",
            "description": "Client-generated challenge identifier reused unchanged when retrying the same application."
          },
          "request": {
            "$ref": "#/components/schemas/NodeChallengeRequest",
            "description": "Registered identity, operation and typed intent digest to bind into the challenge."
          }
        },
        "additionalProperties": false
      },
      "NodeChallengeRequest": {
        "type": "object",
        "description": "Identity and operation binding included in a signed challenge application.",
        "required": [
          "networkId",
          "nodeId",
          "keyId",
          "operation",
          "requestDigest"
        ],
        "properties": {
          "keyId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Current bound control key used to authorize the challenge."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network to which the registered Node is bound."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Registered Node requesting the challenge."
          },
          "operation": {
            "$ref": "#/components/schemas/ProofOperation",
            "description": "Operation for which the challenge may be consumed."
          },
          "requestDigest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Domain-separated SHA-256 digest of the canonical typed operation intent."
          }
        },
        "additionalProperties": false
      },
      "NodeConfiguration": {
        "type": "object",
        "description": "Complete private execution target compiled by the Server for one Node; installation also requires a matching valid lease.",
        "required": [
          "networkId",
          "nodeId",
          "revision",
          "publication",
          "dependencyDigest",
          "enabled",
          "hostname",
          "identity",
          "addresses",
          "selfKeys",
          "meshAddressPools",
          "egressCapture",
          "listeners",
          "dialers",
          "peers",
          "permits",
          "localEgress",
          "diagnostics",
          "decisions",
          "dns",
          "exitEnabled"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Configured Mesh addresses used consistently by DNS and packet admission."
          },
          "decisions": {
            "type": "object",
            "description": "Server compilation results for this Node's live business demands.",
            "additionalProperties": {
              "$ref": "#/components/schemas/DemandDecision"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "dependencyDigest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Digest of the trusted compilation inputs, including demand and identity dependencies."
          },
          "diagnostics": {
            "type": "object",
            "description": "Authenticated measurement only. Never accepted by business packet admission.",
            "additionalProperties": {
              "$ref": "#/components/schemas/DiagnosticPermit"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Leased observation task identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "dialers": {
            "type": "object",
            "description": "Authorized outbound candidates indexed by Server-compiled candidate key.",
            "additionalProperties": {
              "$ref": "#/components/schemas/DialCandidate"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "dns": {
            "$ref": "#/components/schemas/DnsExecution",
            "description": "Compiled DNS settings, visible Mesh names and trusted associations."
          },
          "egressCapture": {
            "$ref": "#/components/schemas/EgressCapturePlan",
            "description": "Disjoint external packet ownership and IPv6 negative capture boundary."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether the source configuration enables this Node's business execution."
          },
          "exitEnabled": {
            "type": "boolean",
            "description": "Configured exit eligibility gated by Node enablement, not proof of installed forwarding."
          },
          "exitRole": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ExitRolePlan",
                "description": "Exit resource responsibility, issued before capability readiness."
              },
              {
                "type": "null"
              }
            ]
          },
          "hostname": {
            "type": "string",
            "description": "Configured canonical hostname, independent of the reported OS hostname."
          },
          "identity": {
            "$ref": "#/components/schemas/NodeIdentity",
            "description": "Configured User or Tag identity used for authorization."
          },
          "listeners": {
            "type": "object",
            "description": "Complete local listener parameters and material indexed by configuration key.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PreparedListener"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "localEgress": {
            "type": "object",
            "description": "Source-only, exact local execution bindings. Never Mesh/path admission.",
            "additionalProperties": {
              "$ref": "#/components/schemas/LocalEgressPermit"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "meshAddressPools": {
            "$ref": "#/components/schemas/AddressPools",
            "description": "Complete managed capture ranges, never peer inventory or business authority."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network for which this target was compiled."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Only Node intended to install this target."
          },
          "peers": {
            "type": "object",
            "description": "Trusted identities and addresses of required execution peers.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PeerConfiguration"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "permits": {
            "type": "object",
            "description": "Includes origin, relay and receiver admission. Absence rejects a new flow.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PathPermit"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "publication": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Execution publication that fixes this target and its dependencies."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Source configuration revision fixed by this publication."
          },
          "selfKeys": {
            "$ref": "#/components/schemas/TrustedNode",
            "description": "This Node's trusted public key bindings and authenticated capabilities."
          }
        },
        "additionalProperties": false
      },
      "NodeConfigurationDelta": {
        "type": "object",
        "description": "Server-computed execution delta reconstructed in a temporary complete target and verified against its digest before installation.",
        "required": [
          "networkId",
          "nodeId",
          "base",
          "target",
          "dependencyDigest",
          "enabled",
          "hostname",
          "identity",
          "addresses",
          "selfKeys",
          "exitEnabled",
          "dns",
          "meshAddressPools",
          "egressCapture",
          "listeners",
          "dialers",
          "peers",
          "permits",
          "decisions",
          "localEgress",
          "diagnostics"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Target's complete configured Mesh address assignment."
          },
          "base": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact saved configuration required as the reconstruction base."
          },
          "decisions": {
            "$ref": "#/components/schemas/DecisionDelta",
            "description": "Source demand decision removals and complete replacements."
          },
          "dependencyDigest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Replacement digest of the target's trusted compilation dependencies."
          },
          "diagnostics": {
            "$ref": "#/components/schemas/DiagnosticDelta",
            "description": "Independent diagnostic permit removals and complete replacements."
          },
          "dialers": {
            "$ref": "#/components/schemas/DialerDelta",
            "description": "Outbound candidate removals and complete replacements."
          },
          "dns": {
            "$ref": "#/components/schemas/DnsExecution",
            "description": "Complete replacement DNS execution content."
          },
          "egressCapture": {
            "$ref": "#/components/schemas/EgressCapturePlan",
            "description": "Complete external packet ownership replacement."
          },
          "enabled": {
            "type": "boolean",
            "description": "Target's configured business execution enablement."
          },
          "exitEnabled": {
            "type": "boolean",
            "description": "Target's configured exit eligibility, gated by Node enablement."
          },
          "exitRole": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ExitRolePlan",
                "description": "Complete exit role replacement; absent withdraws the role."
              },
              {
                "type": "null"
              }
            ]
          },
          "hostname": {
            "type": "string",
            "description": "Target's configured canonical hostname."
          },
          "identity": {
            "$ref": "#/components/schemas/NodeIdentity",
            "description": "Target's complete User or Tag authorization identity."
          },
          "listeners": {
            "$ref": "#/components/schemas/ListenerDelta",
            "description": "Local listener removals and complete replacements."
          },
          "localEgress": {
            "$ref": "#/components/schemas/LocalEgressDelta",
            "description": "Source-only local egress permit removals and complete replacements."
          },
          "meshAddressPools": {
            "$ref": "#/components/schemas/AddressPools",
            "description": "Complete replacement Mesh capture pools, without granting business access."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network shared by the base and target configurations."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Only Node to which this delta applies."
          },
          "peers": {
            "$ref": "#/components/schemas/PeerDelta",
            "description": "Trusted peer removals and complete replacements."
          },
          "permits": {
            "$ref": "#/components/schemas/PermitDelta",
            "description": "Business path permit removals and complete replacements."
          },
          "selfKeys": {
            "$ref": "#/components/schemas/TrustedNode",
            "description": "Target's trusted local public key bindings and authenticated capabilities."
          },
          "target": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact reconstructed target reference, including the digest to verify."
          }
        },
        "additionalProperties": false
      },
      "NodeConfigurationRequest": {
        "type": "object",
        "description": "Instance-bound request to synchronize a complete execution target or explicitly refresh its lease.",
        "required": [
          "instance"
        ],
        "properties": {
          "base": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Exact locally saved base for delta or unchanged detection; null or omission requests a full target when available."
              },
              {
                "type": "null"
              }
            ]
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current runtime instance receiving the target."
          }
        },
        "additionalProperties": false
      },
      "NodeConfigurationResponse": {
        "type": "object",
        "description": "Consistent Server snapshot of the instance lease and its full, delta or unchanged execution target.",
        "required": [
          "lease"
        ],
        "properties": {
          "delta": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NodeConfigurationDelta",
                "description": "Delta from a retained matching base; null when returning a full target, an unchanged target or no target."
              },
              {
                "type": "null"
              }
            ]
          },
          "full": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/CompiledNode",
                "description": "Full replacement or omitted when the exact acknowledged target matches.\nA stale base always receives a complete value, never an unverified patch."
              },
              {
                "type": "null"
              }
            ]
          },
          "lease": {
            "$ref": "#/components/schemas/InstanceLease",
            "description": "Qualification for the exact response target; only explicit renewal operations extend its deadlines."
          }
        },
        "additionalProperties": false
      },
      "NodeConfigurationView": {
        "type": "object",
        "description": "A node's own desired settings and exact current execution reference, without installation materials.",
        "required": [
          "networkId",
          "nodeId",
          "revision",
          "settings"
        ],
        "properties": {
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network to which the authenticated node belongs."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node whose desired settings are projected."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Revision of the desired settings below, not the installable target."
          },
          "settings": {
            "$ref": "#/components/schemas/NodeSettingsView",
            "description": "Redacted desired settings for this node."
          },
          "target": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Exact current execution target. Fetch materials with configuration/query."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "NodeDnsQueryRequest": {
        "type": "object",
        "description": "Exact Mesh DNS name lookup in the authenticated Node's visible configuration.",
        "required": [
          "name"
        ],
        "properties": {
          "name": {
            "type": "string",
            "description": "Exact FQDN; raw UTF-8 is bounded before strict DNS/IDNA normalization.",
            "maxLength": 1024
          }
        },
        "additionalProperties": false
      },
      "NodeDnsQueryResponse": {
        "type": "object",
        "description": "Permission-filtered name lookup snapshot without an execution grant or DNS TTL.",
        "required": [
          "requestId",
          "networkId",
          "sourceNodeId",
          "configurationRevision",
          "asOf",
          "record"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "UTC time at which the Server evaluated the lookup."
          },
          "configurationRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Configuration revision used to resolve and authorize the name."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network determined from the authenticated Node session."
          },
          "record": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/NodeDnsRecord",
                "description": "Unknown and non-visible names have the same null result. This snapshot\ncarries no execution grant, TTL, or authoritative DNS negative answer."
              },
              {
                "type": "null"
              }
            ]
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server request identifier for correlation."
          },
          "sourceNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Authenticated Node whose visibility was used for the lookup."
          }
        },
        "additionalProperties": false
      },
      "NodeDnsRecord": {
        "type": "object",
        "description": "Visible configured Mesh name and addresses for a Node.",
        "required": [
          "nodeId",
          "name",
          "addresses"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Configured Mesh addresses associated with this name."
          },
          "name": {
            "type": "string",
            "description": "Canonical fully qualified Mesh DNS name."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Visible Node identified by the queried name."
          }
        },
        "additionalProperties": false
      },
      "NodeEntitiesView": {
        "type": "object",
        "description": "Node-visible inventory projected under its current authenticated scope.",
        "required": [
          "nodes"
        ],
        "properties": {
          "nodes": {
            "type": "object",
            "description": "Visible node summaries keyed by node ID; presence does not grant business access.",
            "additionalProperties": {
              "$ref": "#/components/schemas/NodeInventoryView"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "NodeEvent": {
        "oneOf": [
          {
            "type": "object",
            "description": "First frame of an initial synchronization batch, containing the visible configuration.",
            "required": [
              "batch",
              "requestId",
              "configuration",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier shared by all snapshots and the ready frame in this batch."
              },
              "configuration": {
                "$ref": "#/components/schemas/NodeConfigurationView",
                "description": "Current configuration projected for the authenticated user or Node."
              },
              "progress": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/ConfigurationApplyProgress",
                    "description": "Visible configuration application progress when available; current Server snapshots include it."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "requestId": {
                "$ref": "#/components/schemas/RequestId",
                "description": "Correlation ID of the HTTP request carrying this stream, as in a\nGET response and the `x-request-id` header."
              },
              "type": {
                "type": "string",
                "enum": [
                  "config.snapshot"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Entity inventory frame of the initial synchronization batch.",
            "required": [
              "batch",
              "entities",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier shared with this batch's configuration, state, and ready frames."
              },
              "entities": {
                "$ref": "#/components/schemas/NodeEntitiesView",
                "description": "Complete entity inventory within the caller's authorized scope."
              },
              "type": {
                "type": "string",
                "enum": [
                  "entities.snapshot"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "State frame of the initial synchronization batch, retaining original report freshness.",
            "required": [
              "batch",
              "state",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier shared with this batch's configuration, entities, and ready frames."
              },
              "state": {
                "$ref": "#/components/schemas/StateView",
                "description": "Current permission-filtered operational state."
              },
              "type": {
                "type": "string",
                "enum": [
                  "state.snapshot"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Commit marker allowing the client to apply a complete initial snapshot batch.",
            "required": [
              "batch",
              "cursor",
              "heartbeatIntervalMs",
              "disconnectAfterMs",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier of the completed snapshot batch."
              },
              "cursor": {
                "type": "string",
                "description": "Resume cursor to save only after the entire batch has been validated and applied."
              },
              "disconnectAfterMs": {
                "type": "integer",
                "format": "int32",
                "description": "Read-idle timeout in milliseconds after which the client should reconnect.",
                "minimum": 0
              },
              "heartbeatIntervalMs": {
                "type": "integer",
                "format": "int32",
                "description": "Expected interval between idle heartbeat comments, in milliseconds.",
                "minimum": 0
              },
              "type": {
                "type": "string",
                "enum": [
                  "sync.ready"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Incremental operational state projection following the previously applied stream cursor.",
            "required": [
              "stateEpoch",
              "baseCursor",
              "cursor",
              "updated",
              "removed",
              "links",
              "paths",
              "type"
            ],
            "properties": {
              "asOf": {
                "type": [
                  "string",
                  "null"
                ],
                "format": "date-time",
                "description": "Projection time, not a new measurement timestamp. Current Server\nframes include these metadata fields; optionality permits consumers\nto apply previously retained deltas without inventing values."
              },
              "baseCursor": {
                "type": "string",
                "description": "Cursor of the preceding applied stream event, including non-state notices."
              },
              "configurationRevision": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/Revision",
                    "description": "Configuration revision used for this projection; omitted in retained deltas lacking this metadata."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "cursor": {
                "type": "string",
                "description": "Resume cursor to save after this delta has been validated and applied."
              },
              "links": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/AggregatedLink"
                },
                "description": "Replacement collection of currently visible aggregated links."
              },
              "paths": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/AggregatedPath"
                },
                "description": "Replacement collection of currently visible aggregated paths."
              },
              "removed": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/NodeId"
                },
                "description": "Node identifiers to remove from the cached state view."
              },
              "sequence": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/Sequence",
                    "description": "Network committed-change sequence; omitted only in retained deltas lacking this metadata."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "stateEpoch": {
                "$ref": "#/components/schemas/StateEpoch",
                "description": "Epoch identifying the state continuity domain for this update."
              },
              "type": {
                "type": "string",
                "enum": [
                  "state.delta"
                ]
              },
              "updated": {
                "type": "object",
                "description": "Complete replacement state for each updated visible node.",
                "additionalProperties": {
                  "$ref": "#/components/schemas/NodeStateView"
                },
                "propertyNames": {
                  "type": "string",
                  "format": "uuid",
                  "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
                  "maxLength": 36,
                  "minLength": 36,
                  "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
                }
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Entity replacements and explicit removals following the previously applied stream cursor.",
            "required": [
              "baseCursor",
              "cursor",
              "updated",
              "removed",
              "type"
            ],
            "properties": {
              "baseCursor": {
                "type": "string",
                "description": "Cursor of the preceding applied stream event, including configuration and resource notices."
              },
              "cursor": {
                "type": "string",
                "description": "Resume cursor to save after this delta has been validated and applied."
              },
              "removed": {
                "$ref": "#/components/schemas/EntityRemovals",
                "description": "Explicit entity identifiers to remove from the cached inventory."
              },
              "type": {
                "type": "string",
                "enum": [
                  "entities.delta"
                ]
              },
              "updated": {
                "$ref": "#/components/schemas/NodeEntitiesView",
                "description": "Updated entity fragments that replace their corresponding cached entries."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Notification of a committed configuration revision, independent of Node application progress.",
            "required": [
              "networkId",
              "previousRevision",
              "revision",
              "changedAt",
              "cause",
              "actor",
              "changedPointers",
              "changesComplete",
              "sourceChanged",
              "type"
            ],
            "properties": {
              "actor": {
                "$ref": "#/components/schemas/EventActor",
                "description": "Committing actor, redacted for users without management scope and for Nodes."
              },
              "cause": {
                "$ref": "#/components/schemas/ConfigurationChangeCause",
                "description": "Primary trigger for the configuration change."
              },
              "changedAt": {
                "type": "string",
                "format": "date-time",
                "description": "Time at which the configuration change was committed."
              },
              "changedPointers": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Visible changed JSON Pointers, provided as invalidation hints rather than a complete dependency analysis."
              },
              "changesComplete": {
                "type": "boolean",
                "description": "Whether the changed-pointer list is complete rather than redacted or truncated."
              },
              "networkId": {
                "$ref": "#/components/schemas/NetworkId",
                "description": "Network whose saved configuration changed."
              },
              "previousRevision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Desired configuration revision immediately before the commit."
              },
              "requestId": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/RequestId",
                    "description": "Committing request's correlation ID, or null when withheld from the subscriber."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "revision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Newly committed desired configuration revision."
              },
              "sourceChanged": {
                "type": "boolean",
                "description": "Whether the original configuration source text changed."
              },
              "type": {
                "type": "string",
                "enum": [
                  "configuration.changed"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Configuration application progress using the same projection as the progress GET operation.",
            "required": [
              "progress",
              "type"
            ],
            "properties": {
              "progress": {
                "$ref": "#/components/schemas/ConfigurationApplyProgress",
                "description": "Updated application progress for the fixed configuration submission target."
              },
              "type": {
                "type": "string",
                "enum": [
                  "configuration.progress"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A manager must re-read the normal resource GET view in this network.\nThis notice carries no source URL, material or unscoped artifact data.",
            "required": [
              "networkId",
              "revision",
              "type"
            ],
            "properties": {
              "networkId": {
                "$ref": "#/components/schemas/NetworkId",
                "description": "Network whose authorized resource view should be reread."
              },
              "revision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Configuration revision associated with the resource change notification."
              },
              "type": {
                "type": "string",
                "enum": [
                  "resources.changed"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Subscriber-specific observation task status update.",
            "required": [
              "observation",
              "type"
            ],
            "properties": {
              "observation": {
                "$ref": "#/components/schemas/ObservationTask",
                "description": "Observation task with this subscription's current status and execution context."
              },
              "type": {
                "type": "string",
                "enum": [
                  "observation.status"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Subscriber-specific observation update containing actual measurement results.",
            "required": [
              "observation",
              "type"
            ],
            "properties": {
              "observation": {
                "$ref": "#/components/schemas/ObservationTask",
                "description": "Observation task with authorized samples and their original measurement context."
              },
              "type": {
                "type": "string",
                "enum": [
                  "observation.sample"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Notification that a terminated observation subscription has left the retained view.",
            "required": [
              "subscriptionId",
              "type"
            ],
            "properties": {
              "subscriptionId": {
                "$ref": "#/components/schemas/ObservationSubscriptionId",
                "description": "Subscription whose cached observation view should be removed."
              },
              "type": {
                "type": "string",
                "enum": [
                  "observation.removed"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Notification that incremental continuity was lost and a new full batch is required.",
            "required": [
              "reason",
              "type"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/ResetReason",
                "description": "Reason the previous cursor and cached view can no longer continue incrementally."
              },
              "type": {
                "type": "string",
                "enum": [
                  "sync.reset"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Final stream notice before closure when an explicit reason can be delivered.",
            "required": [
              "reason",
              "type"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/ClosedReason",
                "description": "Authentication, lease, or shutdown condition that ended the stream."
              },
              "type": {
                "type": "string",
                "enum": [
                  "stream.closed"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "One SSE data frame containing a permission-filtered snapshot, update, or stream lifecycle notice."
      },
      "NodeId": {
        "type": "string",
        "format": "uuid",
        "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "NodeIdentity": {
        "oneOf": [
          {
            "type": "object",
            "description": "Authorize the node as a network user and that user's groups.",
            "required": [
              "user",
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "user"
                ]
              },
              "user": {
                "type": "string",
                "description": "Email of an active network member, bound to stable user and membership IDs in the saved version."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Authorize the node by tags without inheriting user or group permissions.",
            "required": [
              "tags",
              "type"
            ],
            "properties": {
              "tags": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Nonempty, duplicate-free `tag:<name>` references defined by `tagsOwners`."
              },
              "type": {
                "type": "string",
                "enum": [
                  "tag"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Exclusive authorization identity used to match a node against policy selectors."
      },
      "NodeInventoryResponse": {
        "type": "object",
        "description": "Registered Nodes visible under the caller's current network permissions.",
        "required": [
          "items"
        ],
        "properties": {
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeInventoryView"
            },
            "description": "Visible Node inventory entries with current action eligibility."
          }
        },
        "additionalProperties": false
      },
      "NodeInventoryView": {
        "type": "object",
        "description": "Permission-filtered Node registration, configured identity, key metadata and current action eligibility.",
        "required": [
          "id",
          "networkId",
          "registeredAt",
          "hostname",
          "enabled",
          "identity",
          "addresses",
          "keys",
          "controlKeyRevokedAt",
          "dataplaneKeyRevokedAt",
          "canDelete",
          "desiredExecution",
          "canRevokeControlKey",
          "canRevokeDataplaneKey",
          "controlKeyCreatedAt",
          "dataplaneKeyCreatedAt"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Desired Mesh addresses from the current configuration."
          },
          "canDelete": {
            "type": "boolean",
            "description": "Whether the current caller may permanently delete this Node."
          },
          "canRevokeControlKey": {
            "type": "boolean",
            "description": "Whether the current caller may revoke the displayed control key."
          },
          "canRevokeDataplaneKey": {
            "type": "boolean",
            "description": "Whether the current caller may revoke the displayed data-plane key."
          },
          "controlKeyCreatedAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC creation time of the displayed control key binding."
          },
          "controlKeyRevokedAt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Timestamp",
                "description": "Revocation of the displayed latest keys; revoked configured nodes remain manageable."
              },
              {
                "type": "null"
              }
            ]
          },
          "dataplaneKeyCreatedAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC creation time of the displayed data-plane key binding."
          },
          "dataplaneKeyRevokedAt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Timestamp",
                "description": "UTC revocation time of the displayed data-plane key, or null if it is not revoked."
              },
              {
                "type": "null"
              }
            ]
          },
          "desiredExecution": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Current execution target, not a saved/applied receipt. May be a later refresh than the source apply."
              },
              {
                "type": "null"
              }
            ]
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether configuration enables business execution; disabled Nodes may still synchronize control state."
          },
          "hostname": {
            "type": "string",
            "description": "Canonical hostname from the current configuration, not the reported OS hostname."
          },
          "id": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Stable registered Node identifier."
          },
          "identity": {
            "$ref": "#/components/schemas/NodeIdentity",
            "description": "Current configured User or Tag authorization identity."
          },
          "keys": {
            "$ref": "#/components/schemas/NodeKeyMetadata",
            "description": "Public metadata of the latest bound keys."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network to which the Node is registered."
          },
          "registeredAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC time at which the Node was registered."
          }
        },
        "additionalProperties": false
      },
      "NodeKeyMetadata": {
        "type": "object",
        "description": "Public metadata for the Node's separately bound control and data-plane keys.",
        "required": [
          "control",
          "dataplane"
        ],
        "properties": {
          "control": {
            "$ref": "#/components/schemas/BoundNodeKey",
            "description": "Bound Ed25519 control identity key."
          },
          "dataplane": {
            "$ref": "#/components/schemas/BoundNodeKey",
            "description": "Bound X25519 data-plane key."
          }
        },
        "additionalProperties": false
      },
      "NodeObservationCancelled": {
        "type": "object",
        "description": "Server cancellation result indicating whether another subscriber still requires the shared source task.",
        "required": [
          "task"
        ],
        "properties": {
          "task": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ObservationTask",
                "description": "The source executor's still-shared task after this subscription ends;\nnull only when no other consumer remains."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "NodeObservationQuery": {
        "type": "object",
        "description": "Instance qualification required when reading a Node-owned observation subscription.",
        "required": [
          "instanceId",
          "generation"
        ],
        "properties": {
          "generation": {
            "$ref": "#/components/schemas/Version",
            "description": "Server-issued instance generation checked to reject replaced callers."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Current source runtime instance owning the subscription."
          }
        },
        "additionalProperties": false
      },
      "NodeObservationResponse": {
        "type": "object",
        "description": "Server projection of the caller's diagnostic subscription and its resolved configured target.",
        "required": [
          "task",
          "target"
        ],
        "properties": {
          "target": {
            "$ref": "#/components/schemas/NodeObservationTargetView",
            "description": "Configured target resolved and authorized by the Server."
          },
          "task": {
            "$ref": "#/components/schemas/ObservationTask",
            "description": "Shared observation task projected with this subscription's independent deadline and identifier."
          }
        },
        "additionalProperties": false
      },
      "NodeObservationTarget": {
        "oneOf": [
          {
            "type": "object",
            "description": "Select a Node by configured hostname or Mesh FQDN.",
            "required": [
              "name",
              "type"
            ],
            "properties": {
              "name": {
                "type": "string",
                "description": "Requested hostname or network FQDN, limited to 253 characters before resolution.",
                "maxLength": 253
              },
              "type": {
                "type": "string",
                "enum": [
                  "name"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Select the Node assigned an exact Mesh IP address.",
            "required": [
              "address",
              "type"
            ],
            "properties": {
              "address": {
                "type": "string",
                "description": "Requested numeric Mesh IP, limited to 64 characters and not an outer transport-family constraint.",
                "maxLength": 64
              },
              "type": {
                "type": "string",
                "enum": [
                  "meshIp"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Target selector resolved by the Server within the authenticated Node's network and diagnostic permissions."
      },
      "NodeObservationTargetView": {
        "type": "object",
        "description": "Server-resolved configured target returned with an authorized Node observation subscription.",
        "required": [
          "nodeId",
          "hostname",
          "addresses"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Target's configured Mesh addresses from the authorized configuration."
          },
          "hostname": {
            "type": "string",
            "description": "Target's canonical configured hostname."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Registered Node selected by the requested name or Mesh IP."
          }
        },
        "additionalProperties": false
      },
      "NodeProof": {
        "type": "object",
        "description": "Signature proving possession of the control key for an exact challenge transcript.",
        "required": [
          "challengeId",
          "signature"
        ],
        "properties": {
          "challengeId": {
            "$ref": "#/components/schemas/ChallengeId",
            "description": "Identifier of the challenge signed by this proof."
          },
          "signature": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "64-byte Ed25519 signature encoded in unpadded base64url."
          }
        },
        "additionalProperties": false
      },
      "NodePublicKey": {
        "type": "object",
        "description": "Canonical hex key bytes. Algorithms are checked against their purpose at binding.",
        "required": [
          "algorithm",
          "bytes"
        ],
        "properties": {
          "algorithm": {
            "$ref": "#/components/schemas/KeyAlgorithm",
            "description": "Algorithm defining how the public key bytes are interpreted."
          },
          "bytes": {
            "$ref": "#/components/schemas/Digest",
            "description": "Canonical lowercase hexadecimal encoding of the 32 public key bytes."
          }
        },
        "additionalProperties": false
      },
      "NodePublicationProgress": {
        "type": "object",
        "description": "Server-derived receipt projection for one Node's target within a publication.",
        "required": [
          "target",
          "current",
          "saved",
          "applied",
          "failed",
          "freshness",
          "lastReceivedAt"
        ],
        "properties": {
          "applied": {
            "type": "boolean",
            "description": "Whether freshness is current and the latest attempt and sequence report applied."
          },
          "current": {
            "type": "boolean",
            "description": "Whether this publication is still the Node's desired execution publication."
          },
          "failed": {
            "type": "boolean",
            "description": "Whether the latest receipt by attempt and sequence for the current instance reports failure."
          },
          "freshness": {
            "$ref": "#/components/schemas/ReceiptFreshness",
            "description": "Current validity of the target and instance, evaluated independently of receipt phase."
          },
          "lastReceivedAt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Timestamp",
                "description": "Latest Server reception time among this Node's publication receipts, or null when none exist."
              },
              {
                "type": "null"
              }
            ]
          },
          "saved": {
            "type": "boolean",
            "description": "Whether a saved receipt exists for the current instance and generation in this publication."
          },
          "target": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact target assigned to this Node by the publication."
          }
        },
        "additionalProperties": false
      },
      "NodeRotationIntent": {
        "type": "object",
        "description": "Exact replacement keys bound into a Node key-rotation proof.",
        "required": [
          "requestId",
          "controlKey",
          "dataplaneKey"
        ],
        "properties": {
          "controlKey": {
            "$ref": "#/components/schemas/NodePublicKey",
            "description": "New Ed25519 control public key."
          },
          "dataplaneKey": {
            "$ref": "#/components/schemas/NodePublicKey",
            "description": "New independent X25519 data-plane public key."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Stable identifier reused to recover the same completed rotation."
          }
        },
        "additionalProperties": false
      },
      "NodeRotationRequest": {
        "type": "object",
        "description": "Rotation request proving possession of both the old and replacement control keys.",
        "required": [
          "intent",
          "oldProof",
          "newSignature"
        ],
        "properties": {
          "intent": {
            "$ref": "#/components/schemas/NodeRotationIntent",
            "description": "Replacement keys and stable rotation request identifier."
          },
          "newSignature": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "New control key's 64-byte signature over the same challenge, encoded as unpadded base64url."
          },
          "oldProof": {
            "$ref": "#/components/schemas/NodeProof",
            "description": "Existing control key's signature over the rotation challenge."
          }
        },
        "additionalProperties": false
      },
      "NodeSessionIntent": {
        "type": "object",
        "description": "Stable intent used to recover an unexpired Node session after an uncertain response.",
        "required": [
          "requestId"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Client request identifier reused for the same session intent."
          }
        },
        "additionalProperties": false
      },
      "NodeSessionRequest": {
        "type": "object",
        "description": "Session intent and fresh one-time proof submitted by the registered Node.",
        "required": [
          "intent",
          "proof"
        ],
        "properties": {
          "intent": {
            "$ref": "#/components/schemas/NodeSessionIntent",
            "description": "Session intent whose digest is bound by the challenge."
          },
          "proof": {
            "$ref": "#/components/schemas/NodeProof",
            "description": "Control-key signature over the challenge transcript."
          }
        },
        "additionalProperties": false
      },
      "NodeSessionResponse": {
        "type": "object",
        "description": "Short-lived opaque bearer session issued after validating the Node's control-key proof.",
        "required": [
          "nodeId",
          "networkId",
          "keyId",
          "token",
          "expiresAt"
        ],
        "properties": {
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "UTC session deadline, preserved when the same unexpired session is recovered."
          },
          "keyId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Control key whose current binding authorized the session."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network to which the session is restricted."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Registered Node authenticated by this session."
          },
          "token": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Secret bearer credential for subsequent Node requests."
          }
        },
        "additionalProperties": false
      },
      "NodeSettings": {
        "type": "object",
        "description": "Canonical settings for a registered node, including prepared listener materials.",
        "required": [
          "hostname",
          "identity",
          "addresses"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Desired Mesh addresses bound to the same configuration version as identity and DNS."
          },
          "dialers": {
            "$ref": "#/components/schemas/DialerSet",
            "description": "Omission inherits network dialers; an explicit set replaces them completely and `{}` disables active dialing."
          },
          "enabled": {
            "type": "boolean",
            "description": "Enable business execution; defaults to true, while false still permits authenticated configuration synchronization."
          },
          "exitEnabled": {
            "type": "boolean",
            "description": "Permit selection as an exit node; defaults to false and does not indicate execution readiness."
          },
          "hostname": {
            "type": "string",
            "description": "Network-unique DNS label of 1–63 lowercase letters, digits or interior hyphens."
          },
          "identity": {
            "$ref": "#/components/schemas/NodeIdentity",
            "description": "Authoritative user or tag identity for policy matching."
          },
          "listeners": {
            "type": "object",
            "description": "Fixed inbound resources keyed by a nonempty node-local name; defaults to an empty map.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PreparedListener"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "remark": {
            "type": "string",
            "description": "Free-form description, defaulting to an empty string and granting no permissions."
          }
        },
        "additionalProperties": false
      },
      "NodeSettingsPatch": {
        "type": "object",
        "description": "Typed node settings edits; omitted fields retain their current values.",
        "properties": {
          "dialers": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/DialerSetUpdate",
                "description": "Omission retains the override, null restores network inheritance, and an explicit set replaces the protocol selection, with `{}` disabling active dialing."
              },
              {
                "type": "null"
              }
            ]
          },
          "enabled": {
            "type": "boolean",
            "description": "Business execution setting; omission retains it and false withdraws execution without revoking synchronization credentials."
          },
          "exitEnabled": {
            "type": "boolean",
            "description": "Administrative exit eligibility; omission retains the current setting."
          },
          "hostname": {
            "type": "string",
            "description": "Replacement network-unique DNS label; omission retains the current hostname."
          },
          "remark": {
            "type": "string",
            "description": "Replacement description; omission retains it and an empty string clears it."
          }
        },
        "additionalProperties": false
      },
      "NodeSettingsResponse": {
        "type": "object",
        "description": "Non-secret Node settings projected from one configuration revision.",
        "required": [
          "requestId",
          "networkId",
          "nodeId",
          "revision",
          "settings"
        ],
        "properties": {
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network containing the settings."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node whose settings are returned."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server request identifier for correlation."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Configuration revision from which all settings were read."
          },
          "settings": {
            "$ref": "#/components/schemas/NodeSettingsView",
            "description": "Non-secret configured settings visible to the authorized caller."
          }
        },
        "additionalProperties": false
      },
      "NodeSettingsView": {
        "type": "object",
        "description": "Public desired node settings without listener credentials or sensitive header values.",
        "required": [
          "hostname",
          "remark",
          "identity",
          "enabled",
          "addresses",
          "listeners",
          "exitEnabled"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Desired Mesh addresses, with null for an unassigned family."
          },
          "dialers": {
            "$ref": "#/components/schemas/DialerSetView",
            "description": "Omission inherits network dialers; a present set replaces the default protocol selection."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether business execution is administratively enabled, independent of current installation or credential revocation."
          },
          "exitEnabled": {
            "type": "boolean",
            "description": "Administrative exit eligibility, independent of actual forwarding readiness."
          },
          "hostname": {
            "type": "string",
            "description": "Network-unique DNS label used with the network DNS suffix."
          },
          "identity": {
            "$ref": "#/components/schemas/NodeIdentity",
            "description": "Effective user or tag identity used for policy matching."
          },
          "listeners": {
            "type": "object",
            "description": "Redacted fixed inbound resources keyed by node-local listener name.",
            "additionalProperties": {
              "$ref": "#/components/schemas/ListenerView"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "remark": {
            "type": "string",
            "description": "Free-form description with no authorization meaning."
          }
        },
        "additionalProperties": false
      },
      "NodeState": {
        "type": "object",
        "description": "Complete snapshot of a Node's own runtime facts, without granting authority or extending leases.",
        "required": [
          "system",
          "capabilities",
          "configuration",
          "listeners",
          "sockets",
          "links",
          "paths",
          "selection",
          "probes",
          "problems"
        ],
        "properties": {
          "capabilities": {
            "type": "object",
            "description": "Namespaced build capabilities matching the authenticated instance acquisition.",
            "additionalProperties": {
              "$ref": "#/components/schemas/Capability"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationState",
            "description": "Independently reported saved target, applied target and installation progress."
          },
          "exitCapability": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ExitCapability",
                "description": "Target-bound exit prerequisite facts; omitted means unknown, never ready."
              },
              {
                "type": "null"
              }
            ]
          },
          "links": {
            "type": "object",
            "description": "Actual channels grouped by peer, including simultaneous channels to the same peer.",
            "additionalProperties": {
              "type": "array",
              "items": {
                "$ref": "#/components/schemas/LinkState"
              }
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "listeners": {
            "type": "object",
            "description": "Complete listener inventory indexed by configured listener key.",
            "additionalProperties": {
              "$ref": "#/components/schemas/ListenerState"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "paths": {
            "type": "object",
            "description": "Paths observed by this Node, indexed by runtime path identifier.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PathState"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "probes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ProbeObservation"
            },
            "description": "Recent real measurements; repeating a snapshot does not create new samples."
          },
          "problems": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/StateProblem"
            },
            "description": "Current typed failures observed by the Node."
          },
          "selection": {
            "$ref": "#/components/schemas/SelectionState",
            "description": "Routing selection actually applied by the Node."
          },
          "sockets": {
            "type": "object",
            "description": "Complete outer socket inventory indexed by instance-local socket ID.",
            "additionalProperties": {
              "$ref": "#/components/schemas/SocketState"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "system": {
            "$ref": "#/components/schemas/SystemState",
            "description": "Operating system and build facts reported by the Node."
          }
        },
        "additionalProperties": false
      },
      "NodeStateView": {
        "type": "object",
        "description": "Latest visible Node facts with independently derived freshness and IP information.",
        "required": [
          "hostname",
          "freshness",
          "derived"
        ],
        "properties": {
          "derived": {
            "$ref": "#/components/schemas/DerivedNodeState",
            "description": "IP metadata and approximate location derived by the Server from visible observations."
          },
          "freshness": {
            "$ref": "#/components/schemas/Freshness",
            "description": "Current Server evaluation of the report's identity, instance, TTL and configuration binding."
          },
          "hostname": {
            "type": "string",
            "description": "The configured hostname (display name), not the OS hostname a report carries."
          },
          "instanceId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/InstanceId",
                "description": "Instance that produced the accepted report, or null before any report."
              },
              {
                "type": "null"
              }
            ]
          },
          "lastAuthenticatedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Latest authenticated State-report contact, or null before any accepted report."
          },
          "observedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Original Node observation time, or null before any report."
          },
          "receivedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Original Server reception time, or null before any report."
          },
          "report": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/VisibleReport",
                "description": "Permission-filtered accepted report, or null before any report."
              },
              {
                "type": "null"
              }
            ]
          },
          "reportExpiresAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "UTC deadline of the accepted report's freshness, or null before any report."
          },
          "reportSequence": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Sequence",
                "description": "Accepted report sequence, or null before any report."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "NotApplicableReason": {
        "type": "string",
        "description": "Reason public IP metadata lookup does not apply to an address.",
        "enum": [
          "nonPublicAddress",
          "meshAddress"
        ]
      },
      "ObservationEvent": {
        "type": "object",
        "description": "Real Node event retained with its original execution context and occurrence time.",
        "required": [
          "eventId",
          "configuration",
          "occurredAt",
          "payload"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact execution target under which the event occurred."
          },
          "eventId": {
            "$ref": "#/components/schemas/EventId",
            "description": "Stable event identifier used to deduplicate reports from this source Node."
          },
          "occurredAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC occurrence time, unchanged by delivery retries."
          },
          "payload": {
            "$ref": "#/components/schemas/ObservationPayload",
            "description": "Typed event facts reported by the Node's connection or measurement owner."
          }
        },
        "additionalProperties": false
      },
      "ObservationId": {
        "type": "string",
        "format": "uuid",
        "description": "Leased observation task identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "ObservationList": {
        "type": "object",
        "description": "Observation tasks or subscriber projections visible to the authenticated caller.",
        "required": [
          "items"
        ],
        "properties": {
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ObservationTask"
            },
            "description": "Currently visible observation results and lifecycle states."
          }
        },
        "additionalProperties": false
      },
      "ObservationPath": {
        "oneOf": [
          {
            "type": "object",
            "description": "Select an executable complete path from the authorized candidates.",
            "required": [
              "mode"
            ],
            "properties": {
              "mode": {
                "type": "string",
                "enum": [
                  "auto"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Use only the specified complete path without bypassing its required Nodes.",
            "required": [
              "path",
              "mode"
            ],
            "properties": {
              "mode": {
                "type": "string",
                "enum": [
                  "explicit"
                ]
              },
              "path": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/NodeId"
                },
                "description": "Fixed ordered path including the source, required intermediate Nodes and target."
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Path selection constraint for an explicitly authorized diagnostic observation."
      },
      "ObservationPayload": {
        "oneOf": [
          {
            "type": "object",
            "description": "A real connection establishment attempt began.",
            "required": [
              "peerNodeId",
              "attemptId",
              "transport",
              "kind"
            ],
            "properties": {
              "attemptId": {
                "type": "string",
                "description": "Identifier correlating events for the same establishment attempt."
              },
              "kind": {
                "type": "string",
                "enum": [
                  "dial_started"
                ]
              },
              "peerNodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Intended peer of the connection attempt."
              },
              "transport": {
                "type": "string",
                "description": "Transport attempted by the Node."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A real connection establishment attempt failed.",
            "required": [
              "peerNodeId",
              "attemptId",
              "transport",
              "reason",
              "kind"
            ],
            "properties": {
              "attemptId": {
                "type": "string",
                "description": "Identifier of the failed establishment attempt."
              },
              "durationUs": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/Sequence",
                    "description": "Monotonic attempt duration in microseconds, or null when no timing is available."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "kind": {
                "type": "string",
                "enum": [
                  "dial_failed"
                ]
              },
              "peerNodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Intended peer of the failed connection attempt."
              },
              "reason": {
                "$ref": "#/components/schemas/CloseReason",
                "description": "Typed reason the establishment attempt ended."
              },
              "transport": {
                "type": "string",
                "description": "Transport attempted by the Node."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Authentication completed for an actual channel.",
            "required": [
              "peerNodeId",
              "linkId",
              "transport",
              "kind"
            ],
            "properties": {
              "kind": {
                "type": "string",
                "enum": [
                  "link_opened"
                ]
              },
              "linkId": {
                "type": "string",
                "description": "Actual channel identifier bound by the handshake."
              },
              "peerNodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Adjacent Node authenticated on the channel."
              },
              "transport": {
                "type": "string",
                "description": "Transport used by the authenticated channel."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The Node observed an actual channel close.",
            "required": [
              "peerNodeId",
              "linkId",
              "transport",
              "reason",
              "kind"
            ],
            "properties": {
              "kind": {
                "type": "string",
                "enum": [
                  "link_closed"
                ]
              },
              "linkId": {
                "type": "string",
                "description": "Identifier of the channel that closed."
              },
              "peerNodeId": {
                "$ref": "#/components/schemas/NodeId",
                "description": "Adjacent Node of the closed channel."
              },
              "reason": {
                "$ref": "#/components/schemas/CloseReason",
                "description": "Typed reason for the observed closure."
              },
              "transport": {
                "type": "string",
                "description": "Transport used by the closed channel."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A real measured operation reached its reported outcome.",
            "required": [
              "probe",
              "kind"
            ],
            "properties": {
              "kind": {
                "type": "string",
                "enum": [
                  "probe_completed"
                ]
              },
              "probe": {
                "$ref": "#/components/schemas/ProbeObservation",
                "description": "Measurement with its original identity, context and timing boundaries."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The Node's event collector lost observations within a known interval.",
            "required": [
              "dropped",
              "from",
              "to",
              "kind"
            ],
            "properties": {
              "dropped": {
                "$ref": "#/components/schemas/Sequence",
                "description": "Number of events actually dropped, not a count of lost network packets."
              },
              "from": {
                "type": "string",
                "format": "date-time",
                "description": "Original UTC start of the collection gap."
              },
              "kind": {
                "type": "string",
                "enum": [
                  "collection_gap"
                ]
              },
              "to": {
                "type": "string",
                "format": "date-time",
                "description": "Original UTC end of the collection gap."
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Node-reported connection, probe or collection-gap event facts."
      },
      "ObservationPhase": {
        "type": "string",
        "description": "Current task or subscription lifecycle phase, distinct from individual probe outcomes.",
        "enum": [
          "preparing",
          "running",
          "completed",
          "failed",
          "expired",
          "cancelled",
          "unsupported"
        ]
      },
      "ObservationResult": {
        "type": "object",
        "description": "Source Node result bound to the task's actual instance and execution target.",
        "required": [
          "instanceId",
          "configuration",
          "result"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact execution target under which the update was produced."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Source instance that produced the update."
          },
          "result": {
            "$ref": "#/components/schemas/ObservationUpdate",
            "description": "Actual phase or sample reported by the source Node."
          }
        },
        "additionalProperties": false
      },
      "ObservationSample": {
        "type": "object",
        "description": "Actual measurement window; revisions replace the same window rather than adding synthetic samples.",
        "required": [
          "measurementId",
          "version",
          "windowStart",
          "windowEnd",
          "partial"
        ],
        "properties": {
          "measurementId": {
            "$ref": "#/components/schemas/EventId",
            "description": "Stable measurement-window identifier used for deduplication and replacement."
          },
          "partial": {
            "type": "boolean",
            "description": "Whether the window still has incomplete outcomes and may be revised."
          },
          "probe": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ProbeObservation",
                "description": "Actual probe measurement, or null when this sample has no probe result."
              },
              {
                "type": "null"
              }
            ]
          },
          "probeCounts": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ProbeWindow",
                "description": "Sent-probe deadline counts, or null when this metric is unavailable for the sample."
              },
              {
                "type": "null"
              }
            ]
          },
          "traffic": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/TrafficWindow",
                "description": "Business traffic counters, currently null because traffic measurement is unsupported."
              },
              {
                "type": "null"
              }
            ]
          },
          "version": {
            "$ref": "#/components/schemas/Version",
            "description": "Revisions replace a partial window rather than creating another sample."
          },
          "windowEnd": {
            "type": "string",
            "format": "date-time",
            "description": "UTC end of the original measurement window, after its start."
          },
          "windowStart": {
            "type": "string",
            "format": "date-time",
            "description": "UTC start of the original measurement window."
          }
        },
        "additionalProperties": false
      },
      "ObservationServiceScope": {
        "oneOf": [
          {
            "type": "object",
            "description": "Diagnose a path authorized for a specific TCP service.",
            "required": [
              "port",
              "type"
            ],
            "properties": {
              "port": {
                "$ref": "#/components/schemas/Port",
                "description": "TCP destination port used for service authorization."
              },
              "type": {
                "type": "string",
                "enum": [
                  "tcp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Diagnose a path authorized for a specific UDP service.",
            "required": [
              "port",
              "type"
            ],
            "properties": {
              "port": {
                "$ref": "#/components/schemas/Port",
                "description": "UDP destination port used for service authorization."
              },
              "type": {
                "type": "string",
                "enum": [
                  "udp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Diagnose a path authorized for ICMP echo without a transport port.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "icmpEcho"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Exact service scope checked by Access and routing when compiling an independent diagnostic permit."
      },
      "ObservationSubscriptionId": {
        "type": "string",
        "format": "uuid",
        "description": "Independent observation demand and idempotency identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "ObservationTask": {
        "type": "object",
        "description": "Shared diagnostic task with an optional independently leased subscriber projection.",
        "required": [
          "id",
          "sourceNodeId",
          "targetNodeId",
          "service",
          "selection",
          "configuration",
          "instanceId",
          "metrics",
          "intervalMs",
          "phase",
          "unsupportedMetrics",
          "expiresAt",
          "renewAfter"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact source execution target to which results must remain bound."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC task deadline beyond which execution and results are no longer authorized."
          },
          "id": {
            "$ref": "#/components/schemas/ObservationId",
            "description": "Shared Node measurement task identifier, distinct from a subscriber identifier."
          },
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Source runtime instance authorized to execute this task."
          },
          "intervalMs": {
            "type": "integer",
            "format": "int32",
            "description": "Actual sampling interval in milliseconds after budget constraints.",
            "minimum": 0
          },
          "latest": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ObservationSample",
                "description": "Latest accepted real measurement window, or null before a valid sample is available."
              },
              {
                "type": "null"
              }
            ]
          },
          "metrics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MetricKind"
            },
            "description": "Measurement kinds assigned to this task."
          },
          "path": {
            "type": [
              "array",
              "null"
            ],
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Actual complete measured path, or null until a path is selected and reported."
          },
          "pathSegment": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Sequence",
                "description": "Actual path segment identifier, or null before a measured segment is reported."
              },
              {
                "type": "null"
              }
            ]
          },
          "phase": {
            "$ref": "#/components/schemas/ObservationPhase",
            "description": "Current task or subscriber lifecycle phase."
          },
          "renewAfter": {
            "type": "string",
            "format": "date-time",
            "description": "UTC time after which the subscriber should renew under current permissions."
          },
          "selection": {
            "$ref": "#/components/schemas/ObservationPath",
            "description": "Authorized path-selection mode and any fixed complete path."
          },
          "service": {
            "$ref": "#/components/schemas/ObservationServiceScope",
            "description": "Exact service scope covered by the diagnostic permission."
          },
          "sourceNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node authorized to execute and report the measurement."
          },
          "subscriberExpiresAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Independent subscriber's UTC lease deadline, or null in a Node task projection."
          },
          "subscriptionId": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ObservationSubscriptionId",
                "description": "Subscriber identifier for this projection, or null when returning the shared Node task."
              },
              {
                "type": "null"
              }
            ]
          },
          "targetNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Authorized business destination of the measured path."
          },
          "unsupportedMetrics": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MetricKind"
            },
            "description": "Requested metrics the Server cannot provide; traffic is currently unsupported."
          }
        },
        "additionalProperties": false
      },
      "ObservationUpdate": {
        "oneOf": [
          {
            "type": "object",
            "description": "Report lifecycle progress without inventing a path or empty sample.",
            "required": [
              "phase",
              "type"
            ],
            "properties": {
              "phase": {
                "$ref": "#/components/schemas/ReportedObservationPhase",
                "description": "Lifecycle phase actually observed by the source executor."
              },
              "type": {
                "type": "string",
                "enum": [
                  "phase"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Report a nonempty measurement window for an actual path segment.",
            "required": [
              "path",
              "pathSegment",
              "sample",
              "type"
            ],
            "properties": {
              "path": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/NodeId"
                },
                "description": "Actual complete ordered path used for this measurement."
              },
              "pathSegment": {
                "$ref": "#/components/schemas/Sequence",
                "description": "Segment identifier separating results across actual path selections."
              },
              "sample": {
                "$ref": "#/components/schemas/ObservationSample",
                "description": "Actual measurement window with its stable identifier and revision."
              },
              "type": {
                "type": "string",
                "enum": [
                  "sample"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Source Node update separating lifecycle progress from real measured windows."
      },
      "OuterSocket": {
        "type": "string",
        "description": "Outer transport socket classification; never a business access decision.",
        "enum": [
          "udp",
          "tcp"
        ]
      },
      "PasswordLengthUnit": {
        "type": "string",
        "description": "Unit used to measure password length before UTF-8 byte limits are applied.",
        "enum": [
          "unicodeCodepoints"
        ]
      },
      "PasswordResetInspectRequest": {
        "type": "object",
        "description": "Secret-bearing request to inspect a password reset without consuming it.",
        "required": [
          "token"
        ],
        "properties": {
          "token": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "One-time password reset credential issued by an authorized host operator."
          }
        },
        "additionalProperties": false
      },
      "PasswordResetInspection": {
        "type": "object",
        "description": "Validity information for an inspected password reset credential.",
        "required": [
          "requestId",
          "expiresAt"
        ],
        "properties": {
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the reset credential becomes invalid."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "PasswordResetRedeemRequest": {
        "type": "object",
        "description": "One-time password reset redemption that invalidates old user sessions.",
        "required": [
          "token",
          "password"
        ],
        "properties": {
          "password": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Replacement password satisfying the advertised password rules."
          },
          "token": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Unused, unexpired password reset credential for the account's current credential version."
          }
        },
        "additionalProperties": false
      },
      "PasswordRules": {
        "type": "object",
        "description": "Password limits applied without trimming or rewriting the supplied value.",
        "required": [
          "minCodepoints",
          "maxCodepoints",
          "maxUtf8Bytes",
          "lengthUnit"
        ],
        "properties": {
          "lengthUnit": {
            "$ref": "#/components/schemas/PasswordLengthUnit",
            "description": "Counting unit for the minimum and maximum length."
          },
          "maxCodepoints": {
            "type": "integer",
            "format": "int32",
            "description": "Maximum number of Unicode scalar values.",
            "maximum": 65535,
            "minimum": 0
          },
          "maxUtf8Bytes": {
            "type": "integer",
            "format": "int32",
            "description": "Maximum size of the password encoded as UTF-8, in bytes.",
            "maximum": 65535,
            "minimum": 0
          },
          "minCodepoints": {
            "type": "integer",
            "format": "int32",
            "description": "Minimum number of Unicode scalar values.",
            "maximum": 65535,
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "PathEvidence": {
        "type": "string",
        "description": "Stage of path use actually observable by the reporting Node.",
        "enum": [
          "selected",
          "sent",
          "peerConfirmed",
          "delivered"
        ]
      },
      "PathPermit": {
        "type": "object",
        "description": "Server-compiled business authority shared by the source, relays and terminal of each allowed complete path.",
        "required": [
          "demandId",
          "source",
          "sourceAddresses",
          "target",
          "protocol",
          "profile",
          "paths",
          "expiresAt",
          "replies"
        ],
        "properties": {
          "demandId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Business demand for which this permit was compiled."
          },
          "exit": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ExitPermit",
                "description": "Required execution constraints for an external exit; absent for Mesh services."
              },
              {
                "type": "null"
              }
            ]
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Permit deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and bounded by demand and dependency deadlines."
          },
          "paths": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/LogicalPath"
            },
            "description": "Allowed complete paths; a flow retains its established path."
          },
          "port": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Port",
                "description": "Exact TCP or UDP destination port; null or omitted only for ICMP echo."
              },
              {
                "type": "null"
              }
            ]
          },
          "profile": {
            "type": "string",
            "description": "Source routing profile used to compile the permit."
          },
          "protocol": {
            "$ref": "#/components/schemas/BusinessProtocol",
            "description": "Authorized inner business protocol."
          },
          "replies": {
            "$ref": "#/components/schemas/ReplyPolicy",
            "description": "Always reverse the established complete path, never evaluate reverse routing for replies."
          },
          "source": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Authenticated original business source, unchanged across path segments."
          },
          "sourceAddresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Configured source Mesh addresses checked during packet admission."
          },
          "target": {
            "$ref": "#/components/schemas/BusinessTarget",
            "description": "Authorized business destination after Mesh address classification."
          }
        },
        "additionalProperties": false
      },
      "PathState": {
        "type": "object",
        "description": "A participating Node's observation of a complete runtime path.",
        "required": [
          "nodes",
          "publicationId",
          "reporterRole",
          "evidence",
          "observedAt",
          "expiresAt"
        ],
        "properties": {
          "evidence": {
            "$ref": "#/components/schemas/PathEvidence",
            "description": "Stage of path use observed by this reporter."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC deadline after which this path observation is no longer current."
          },
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Actual complete ordered Node path."
          },
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC observation time."
          },
          "publicationId": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Execution publication under which this path was used."
          },
          "reporterRole": {
            "$ref": "#/components/schemas/ReporterRole",
            "description": "Reporting Node's role in the path."
          }
        },
        "additionalProperties": false
      },
      "PeerCandidate": {
        "type": "object",
        "description": "Peer-visible endpoint candidate with its original observation lifetime.",
        "required": [
          "address",
          "source",
          "observedAt",
          "expiresAt"
        ],
        "properties": {
          "address": {
            "type": "string",
            "description": "Candidate host and port, without asserting authenticated reachability."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC deadline after which the candidate observation is no longer current."
          },
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC observation time retained from the Node report."
          },
          "source": {
            "$ref": "#/components/schemas/CandidateSource",
            "description": "Coarse provenance of the candidate."
          }
        },
        "additionalProperties": false
      },
      "PeerConfiguration": {
        "type": "object",
        "description": "Trusted identity and addresses of a peer required by the compiled execution paths.",
        "required": [
          "identity",
          "addresses",
          "controlKey",
          "dataplaneKey"
        ],
        "properties": {
          "addresses": {
            "$ref": "#/components/schemas/NodeAddresses",
            "description": "Peer Mesh addresses used by path and packet admission."
          },
          "controlKey": {
            "$ref": "#/components/schemas/BoundNodeKey",
            "description": "Peer Ed25519 key binding used for control identity proofs."
          },
          "dataplaneKey": {
            "$ref": "#/components/schemas/BoundNodeKey",
            "description": "Peer X25519 key binding used for end-to-end protection."
          },
          "identity": {
            "$ref": "#/components/schemas/NodeIdentity",
            "description": "Peer authorization identity from the same source configuration."
          }
        },
        "additionalProperties": false
      },
      "PeerDelta": {
        "type": "object",
        "description": "Server-computed keyed changes between two complete execution targets.",
        "required": [
          "remove",
          "upsert"
        ],
        "properties": {
          "remove": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Keys present in the base that must be removed from the reconstructed target."
          },
          "upsert": {
            "type": "object",
            "description": "Complete values to insert or replace; keys cannot also occur in remove.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PeerConfiguration"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "PeerEndpoint": {
        "type": "object",
        "description": "Authorized peer endpoint projection retaining its exact runtime resource binding.",
        "required": [
          "socketId",
          "resource",
          "generation",
          "network",
          "candidate",
          "candidateReference"
        ],
        "properties": {
          "candidate": {
            "$ref": "#/components/schemas/PeerCandidate",
            "description": "Visible candidate with source details reduced to their provenance kind."
          },
          "candidateReference": {
            "$ref": "#/components/schemas/CandidateReference",
            "description": "Exact candidate binding used by coordinated connection acquisition."
          },
          "generation": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Socket resource generation used to reject stale bindings."
          },
          "network": {
            "$ref": "#/components/schemas/SocketNetwork",
            "description": "Actual outer socket protocol."
          },
          "resource": {
            "$ref": "#/components/schemas/ResourceRef",
            "description": "Resource owning the reported socket."
          },
          "socketId": {
            "type": "string",
            "description": "Socket identifier within the reporting instance."
          }
        },
        "additionalProperties": false
      },
      "PeerState": {
        "type": "object",
        "description": "Restricted peer facts needed for authorized execution dependencies.",
        "required": [
          "capabilities",
          "configuration",
          "endpoints"
        ],
        "properties": {
          "capabilities": {
            "type": "object",
            "description": "Reported build capabilities, empty when the source report is not current.",
            "additionalProperties": {
              "$ref": "#/components/schemas/Capability"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationState",
            "description": "Peer-reported saved, applied and progress references."
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PeerEndpoint"
            },
            "description": "Currently visible connection candidates with their runtime bindings."
          }
        },
        "additionalProperties": false
      },
      "PermitDelta": {
        "type": "object",
        "description": "Server-computed keyed changes between two complete execution targets.",
        "required": [
          "remove",
          "upsert"
        ],
        "properties": {
          "remove": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/RequestId"
            },
            "description": "Keys present in the base that must be removed from the reconstructed target."
          },
          "upsert": {
            "type": "object",
            "description": "Complete values to insert or replace; keys cannot also occur in remove.",
            "additionalProperties": {
              "$ref": "#/components/schemas/PathPermit"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "PlannedEdge": {
        "type": "object",
        "description": "One hop of a planned path with the topology rule that allows it.",
        "required": [
          "a",
          "b",
          "topology"
        ],
        "properties": {
          "a": {
            "$ref": "#/components/schemas/NodeId",
            "description": "The hop's endpoints in path order."
          },
          "b": {
            "$ref": "#/components/schemas/NodeId"
          },
          "topology": {
            "type": [
              "string",
              "null"
            ],
            "description": "`/topology/<index>` of the allowing rule, null for `defaults.topology`."
          }
        },
        "additionalProperties": false
      },
      "PlannedPath": {
        "type": "object",
        "description": "A complete path from the source to a target, every hop allowed by topology and reachable by a transport.",
        "required": [
          "nodes",
          "edges"
        ],
        "properties": {
          "edges": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/PlannedEdge"
            },
            "description": "Consecutive node pairs of `nodes` with their topology decisions."
          },
          "nodes": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Nodes from the source to the target, in order."
          }
        },
        "additionalProperties": false
      },
      "Port": {
        "type": "integer",
        "format": "int32",
        "description": "A nonzero TCP/UDP port. Successful validation says nothing about availability.",
        "maximum": 65535,
        "minimum": 1
      },
      "PositiveDuration": {
        "type": "string",
        "description": "Positive canonical decimal with ms/s/m/h; total must fit u64 milliseconds. Unit spelling is preserved. Consumers enforce resource and lease limits separately.",
        "pattern": "^(?:(?:[1-9][0-9]{0,18}|1[0-7][0-9]{18}|18[0-3][0-9]{17}|184[0-3][0-9]{16}|1844[0-5][0-9]{15}|18446[0-6][0-9]{14}|184467[0-3][0-9]{13}|1844674[0-3][0-9]{12}|184467440[0-6][0-9]{10}|1844674407[0-2][0-9]{9}|18446744073[0-6][0-9]{8}|1844674407370[0-8][0-9]{6}|18446744073709[0-4][0-9]{5}|184467440737095[0-4][0-9]{4}|1844674407370955[0-0][0-9]{3}|18446744073709551[0-5][0-9]{2}|184467440737095516[0-0][0-9]{1}|1844674407370955161[0-4][0-9]{0}|18446744073709551615)ms|(?:[1-9][0-9]{0,15}|1[0-7][0-9]{15}|18[0-3][0-9]{14}|184[0-3][0-9]{13}|1844[0-5][0-9]{12}|18446[0-6][0-9]{11}|184467[0-3][0-9]{10}|1844674[0-3][0-9]{9}|184467440[0-6][0-9]{7}|1844674407[0-2][0-9]{6}|18446744073[0-6][0-9]{5}|1844674407370[0-8][0-9]{3}|18446744073709[0-4][0-9]{2}|184467440737095[0-4][0-9]{1}|1844674407370955[0-0][0-9]{0}|18446744073709551)s|(?:[1-9][0-9]{0,13}|[1-2][0-9]{14}|30[0-6][0-9]{12}|307[0-3][0-9]{11}|3074[0-3][0-9]{10}|30744[0-4][0-9]{9}|307445[0-6][0-9]{8}|3074457[0-2][0-9]{7}|30744573[0-3][0-9]{6}|307445734[0-4][0-9]{5}|3074457345[0-5][0-9]{4}|30744573456[0-0][0-9]{3}|307445734561[0-7][0-9]{2}|3074457345618[0-1][0-9]{1}|30744573456182[0-4][0-9]{0}|307445734561825)m|(?:[1-9][0-9]{0,11}|[1-4][0-9]{12}|5[0-0][0-9]{11}|51[0-1][0-9]{10}|512[0-3][0-9]{9}|51240[0-8][0-9]{7}|512409[0-4][0-9]{6}|5124095[0-4][0-9]{5}|51240955[0-6][0-9]{4}|512409557[0-5][0-9]{3}|51240955760[0-2][0-9]{1}|5124095576030)h)(?![\\s\\S])"
      },
      "PreparationKind": {
        "oneOf": [
          {
            "type": "string",
            "enum": [
              "addressPoolV4",
              "addressPoolV6",
              "addressV4",
              "addressV6",
              "identityProof",
              "ruleSetContent"
            ]
          },
          {
            "type": "object",
            "required": [
              "material"
            ],
            "properties": {
              "material": {
                "$ref": "#/components/schemas/MaterialRequirement"
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "What a draft still needs before it can be applied: an address pool or address allocation, a device identity proof, remote rule-set content, or `{ material }` naming a listener credential or TLS material to generate or verify."
      },
      "PreparationRequirement": {
        "type": "object",
        "description": "Preparation work associated with an input configuration location.",
        "required": [
          "path",
          "kind"
        ],
        "properties": {
          "kind": {
            "$ref": "#/components/schemas/PreparationKind",
            "description": "Material, address, identity or rule-set dependency that is not yet prepared."
          },
          "path": {
            "type": "string",
            "description": "JSON Pointer to the declaration requiring preparation, such as a listener or address family."
          }
        },
        "additionalProperties": false
      },
      "PrepareUdpTraversalRequest": {
        "type": "object",
        "description": "Node request for an immutable short-lived offer bound to an exact authorized UDP carrier and candidate pair.",
        "required": [
          "instance",
          "configuration",
          "stateEpoch",
          "requestId",
          "expiresAt",
          "peerId",
          "carrier",
          "localCandidate",
          "peerCandidate"
        ],
        "properties": {
          "carrier": {
            "$ref": "#/components/schemas/Carrier",
            "description": "Exact Native UDP carrier authorized by both participants' complete-path permits."
          },
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Source's exact current execution target."
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Absolute requested deadline, unchanged on retry, never a duration."
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current source runtime instance preparing the attempt."
          },
          "localCandidate": {
            "$ref": "#/components/schemas/CandidateReference",
            "description": "Original State-owned candidate from the preparing Node's actual socket."
          },
          "peerCandidate": {
            "$ref": "#/components/schemas/CandidateReference",
            "description": "Original State-owned candidate from the peer's actual socket."
          },
          "peerId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Adjacent peer participating in the authorized UDP acquisition."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Stable identifier reused with identical immutable input when retrying preparation."
          },
          "stateEpoch": {
            "$ref": "#/components/schemas/StateEpoch",
            "description": "State epoch obtained from current coordination metadata."
          }
        },
        "additionalProperties": false
      },
      "PrepareUdpTraversalResponse": {
        "oneOf": [
          {
            "type": "object",
            "description": "A fixed offer is available for bilateral acceptance.",
            "required": [
              "offer",
              "status"
            ],
            "properties": {
              "offer": {
                "$ref": "#/components/schemas/UdpTraversalOffer",
                "description": "Original offer, including its unchanged deadline on an identical retry."
              },
              "status": {
                "type": "string",
                "enum": [
                  "offer"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Acquisition cannot be prepared under the supplied bindings or current limits.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/UdpTraversalUnavailableReason",
                "description": "Closed coordination condition that prevented preparation."
              },
              "status": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Server result of preparing or recovering an immutable UDP acquisition offer."
      },
      "PreparedListener": {
        "oneOf": [
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "type": {
                "type": "string",
                "enum": [
                  "native_udp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "type"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "type": {
                "type": "string",
                "enum": [
                  "native_tcp"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "tls",
              "type",
              "uuid"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "flow": {
                "type": "string",
                "description": "VLESS flow selection; only the empty string is accepted and omission defaults to it."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "stream": {
                "$ref": "#/components/schemas/VlessStream",
                "description": "VLESS encapsulation, `tcp` or `websocket`, both over TLS; defaults to `tcp`."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "tls": {
                "type": "object",
                "description": "TLS server parameters and certificate material for a VLESS or Trojan listener.",
                "required": [
                  "serverName",
                  "certificate",
                  "privateKey"
                ],
                "properties": {
                  "alpn": {
                    "type": "array",
                    "items": {
                      "type": "string"
                    },
                    "description": "Ordered ALPN names of 1–255 bytes without control characters; defaults to empty, and WebSocket permits only `http/1.1` entries."
                  },
                  "certificate": {
                    "$ref": "#/components/schemas/SecretString",
                    "description": "Nonempty PEM certificate chain, leaf first; required in provided mode, otherwise omission generates or retains saved material and null is rejected."
                  },
                  "handshakeTimeout": {
                    "$ref": "#/components/schemas/PositiveDuration",
                    "description": "Positive TLS handshake budget; defaults to `5s` and cannot extend the overall connection or remaining lease budget."
                  },
                  "maxVersion": {
                    "$ref": "#/components/schemas/TlsVersion",
                    "description": "Maximum accepted TLS version, `1.2` or `1.3`; defaults to `1.3` and must not precede minVersion."
                  },
                  "minVersion": {
                    "$ref": "#/components/schemas/TlsVersion",
                    "description": "Minimum accepted TLS version, `1.2` or `1.3`; defaults to `1.2` and must not exceed maxVersion."
                  },
                  "mode": {
                    "$ref": "#/components/schemas/TlsMode",
                    "description": "Certificate-material source; defaults to `managed`, while `provided` requires both certificate and private key."
                  },
                  "privateKey": {
                    "$ref": "#/components/schemas/SecretString",
                    "description": "Nonempty matching PEM private key; required in provided mode, otherwise omission generates or retains saved material and null is rejected."
                  },
                  "serverName": {
                    "type": "string",
                    "description": "Required TLS host name or IP address used for certificate identity and the dialer's verification name."
                  }
                },
                "additionalProperties": false
              },
              "type": {
                "type": "string",
                "enum": [
                  "vless"
                ]
              },
              "uuid": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Hyphenated UUID credential with case-insensitive hexadecimal digits; omission makes the Server generate or retain saved material, while null is rejected."
              },
              "websocket": {
                "$ref": "#/components/schemas/ListenerWebSocket",
                "description": "WebSocket Upgrade settings; omission uses defaults for a websocket stream, while a tcp stream forbids this field and null is rejected."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
            "required": [
              "listenPort",
              "tls",
              "type",
              "password"
            ],
            "properties": {
              "bindInterface": {
                "type": "string",
                "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
              },
              "enabled": {
                "type": "boolean",
                "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
              },
              "endpoints": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/Endpoint"
                },
                "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
              },
              "listen": {
                "type": "string",
                "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
              },
              "listenPort": {
                "$ref": "#/components/schemas/Port",
                "description": "Required local listening port in the range 1–65535."
              },
              "password": {
                "$ref": "#/components/schemas/SecretString",
                "description": "Nonempty password preserved without trimming or case normalization; omission makes the Server generate or retain saved material, while null is rejected."
              },
              "remark": {
                "type": "string",
                "description": "Descriptive text; defaults to an empty string and cannot be null."
              },
              "socket": {
                "$ref": "#/components/schemas/SocketOptions",
                "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
              },
              "stream": {
                "$ref": "#/components/schemas/TrojanStream",
                "description": "Trojan encapsulation; only `tcp` is accepted and omission defaults to it."
              },
              "tcp": {
                "$ref": "#/components/schemas/TcpOptions",
                "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
              },
              "tls": {
                "type": "object",
                "description": "TLS server parameters and certificate material for a VLESS or Trojan listener.",
                "required": [
                  "serverName",
                  "certificate",
                  "privateKey"
                ],
                "properties": {
                  "alpn": {
                    "type": "array",
                    "items": {
                      "type": "string"
                    },
                    "description": "Ordered ALPN names of 1–255 bytes without control characters; defaults to empty, and WebSocket permits only `http/1.1` entries."
                  },
                  "certificate": {
                    "$ref": "#/components/schemas/SecretString",
                    "description": "Nonempty PEM certificate chain, leaf first; required in provided mode, otherwise omission generates or retains saved material and null is rejected."
                  },
                  "handshakeTimeout": {
                    "$ref": "#/components/schemas/PositiveDuration",
                    "description": "Positive TLS handshake budget; defaults to `5s` and cannot extend the overall connection or remaining lease budget."
                  },
                  "maxVersion": {
                    "$ref": "#/components/schemas/TlsVersion",
                    "description": "Maximum accepted TLS version, `1.2` or `1.3`; defaults to `1.3` and must not precede minVersion."
                  },
                  "minVersion": {
                    "$ref": "#/components/schemas/TlsVersion",
                    "description": "Minimum accepted TLS version, `1.2` or `1.3`; defaults to `1.2` and must not exceed maxVersion."
                  },
                  "mode": {
                    "$ref": "#/components/schemas/TlsMode",
                    "description": "Certificate-material source; defaults to `managed`, while `provided` requires both certificate and private key."
                  },
                  "privateKey": {
                    "$ref": "#/components/schemas/SecretString",
                    "description": "Nonempty matching PEM private key; required in provided mode, otherwise omission generates or retains saved material and null is rejected."
                  },
                  "serverName": {
                    "type": "string",
                    "description": "Required TLS host name or IP address used for certificate identity and the dialer's verification name."
                  }
                },
                "additionalProperties": false
              },
              "type": {
                "type": "string",
                "enum": [
                  "trojan"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "An inbound listener as saved: `type` selects the carrier, and credential and TLS materials are complete. Returned by configuration reads."
      },
      "PreparedRuleSet": {
        "type": "object",
        "description": "Transient validated rule-set content summary produced by a noncommitting configuration preview.",
        "required": [
          "digest",
          "entries"
        ],
        "properties": {
          "digest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Digest of the complete normalized destination content, without reserving an artifact ID."
          },
          "entries": {
            "type": "integer",
            "format": "int32",
            "description": "Total normalized CIDR, suffix and exact-name entries.",
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "ProbeChannel": {
        "type": "object",
        "description": "Physical channel actually used for one side of a measured probe exchange.",
        "required": [
          "linkId",
          "socketId",
          "peerNodeId",
          "transport",
          "localAddress",
          "remoteAddress",
          "acquisition"
        ],
        "properties": {
          "acquisition": {
            "$ref": "#/components/schemas/LinkAcquisition",
            "description": "How the executor acquired this physical channel."
          },
          "linkId": {
            "$ref": "#/components/schemas/Digest",
            "description": "Non-secret authenticated channel identifier observed by the executor."
          },
          "localAddress": {
            "$ref": "#/components/schemas/SocketAddress",
            "description": "Local socket address used by this channel."
          },
          "peerNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Adjacent Node authenticated on this physical channel."
          },
          "remoteAddress": {
            "$ref": "#/components/schemas/SocketAddress",
            "description": "Remote socket address used by this channel."
          },
          "socketId": {
            "$ref": "#/components/schemas/Digest",
            "description": "Actual local socket identifier carrying the probe."
          },
          "transport": {
            "$ref": "#/components/schemas/TransportProtocol",
            "description": "Actual outer transport used by the channel."
          }
        },
        "additionalProperties": false
      },
      "ProbeChannels": {
        "type": "object",
        "description": "Actual request and reply channel evidence supplied by the probe executor.",
        "required": [
          "request",
          "reply"
        ],
        "properties": {
          "reply": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ProbeChannel",
                "description": "Null until a reply has actually been observed on a channel."
              },
              {
                "type": "null"
              }
            ]
          },
          "request": {
            "$ref": "#/components/schemas/ProbeChannel",
            "description": "Physical channel used to send the measured request."
          }
        },
        "additionalProperties": false
      },
      "ProbeKind": {
        "type": "string",
        "description": "Operation measured by a probe; different kinds have different timing boundaries.",
        "enum": [
          "neighbor",
          "path",
          "connect",
          "controller"
        ]
      },
      "ProbeObservation": {
        "type": "object",
        "description": "Node-reported measurement with original timing and execution context.",
        "required": [
          "measurementId",
          "kind",
          "path",
          "publicationId",
          "startedAt",
          "outcome"
        ],
        "properties": {
          "channels": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ProbeChannels",
                "description": "Actual physical request/reply channels, when measured by the executor."
              },
              {
                "type": "null"
              }
            ]
          },
          "completedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "UTC completion time, or null while no completion is recorded."
          },
          "durationUs": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Sequence",
                "description": "Monotonic elapsed microseconds, or null when no real timing is available."
              },
              {
                "type": "null"
              }
            ]
          },
          "kind": {
            "$ref": "#/components/schemas/ProbeKind",
            "description": "Measured operation and its timing scope."
          },
          "linkId": {
            "type": [
              "string",
              "null"
            ],
            "description": "Measured channel identifier, or null when no channel is associated."
          },
          "measurementId": {
            "$ref": "#/components/schemas/EventId",
            "description": "Stable measurement identifier reused when the same observation is reported again."
          },
          "outcome": {
            "$ref": "#/components/schemas/ProbeOutcome",
            "description": "Actual outcome of the measured operation."
          },
          "path": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Ordered Node path associated with the measured operation."
          },
          "publicationId": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Execution publication associated with the measurement."
          },
          "startedAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC start time used to place the measurement in history."
          },
          "transport": {
            "type": [
              "string",
              "null"
            ],
            "description": "Measured transport, or null when no transport is associated."
          }
        },
        "additionalProperties": false
      },
      "ProbeOutcome": {
        "type": "string",
        "description": "Actual outcome of a measured operation, without replacing missing timing with zero.",
        "enum": [
          "pending",
          "success",
          "timeout",
          "failed"
        ]
      },
      "ProbeWindow": {
        "type": "object",
        "description": "Counts of actually sent probes and their deadline outcomes within a measurement window.",
        "required": [
          "sent",
          "repliedWithinDeadline",
          "timeout",
          "pending"
        ],
        "properties": {
          "pending": {
            "type": "integer",
            "format": "int32",
            "description": "Sent probes still awaiting a final deadline outcome, excluded from the unanswered-rate denominator.",
            "minimum": 0
          },
          "repliedWithinDeadline": {
            "type": "integer",
            "format": "int32",
            "description": "Sent probes whose replies arrived within their deadline.",
            "minimum": 0
          },
          "sent": {
            "type": "integer",
            "format": "int32",
            "description": "Number of probes actually sent, excluding preparation and unsent local failures.",
            "minimum": 0
          },
          "timeout": {
            "type": "integer",
            "format": "int32",
            "description": "Sent probes that reached their deadline without an on-time reply.",
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "Problem": {
        "oneOf": [
          {
            "type": "object",
            "description": "The supplied login or Node credentials fail authentication; returns HTTP 401 without disclosing credential details.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_credentials"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The supplied current password is incorrect; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_current_password"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The password recovery credential is invalid or no longer usable; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_password_reset"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The verified password recovery credential has already been consumed; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "password_reset_used"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The verified password recovery credential has reached its expiry time; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "password_reset_expired"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The verified password recovery credential was superseded or its account credential version changed; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "password_reset_replaced"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The session token is missing, unknown, expired, revoked, or otherwise invalid; returns HTTP 401 and requires signing in again.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_session"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Public account creation is disabled by the Server; returns HTTP 403.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "registration_disabled"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The requested email address is already registered; returns HTTP 409 without disclosing account details.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "email_taken"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The proposed password fails the published rules; returns HTTP 422 with PasswordRules in params.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "password_weak"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/PasswordRules",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The request JSON, parameters, or input values are invalid; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_request"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The request body exceeds the allowed size; returns HTTP 413.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "request_too_large"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The request does not supply the required JSON content type; returns HTTP 415.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "unsupported_media_type"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The Server cannot admit work within its current computation budget; returns HTTP 503 with Retry-After: 1, so retry later.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "busy"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A required dependency, runtime, storage operation, or internal Server operation is unavailable or failed; returns HTTP 503.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "service_unavailable"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The requested resource or route cannot be found; returns HTTP 404.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "not_found"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The requested HTTP method is not supported by the route; returns HTTP 405.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "method_not_allowed"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The authenticated principal lacks permission for the requested operation; returns HTTP 403.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "permission_denied"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A request ID was already used with different input; returns HTTP 409 independently of generation, binding, or revision conflicts.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "request_id_conflict"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The request does not match the current generation or binding; returns HTTP 409 rather than indicating request-ID reuse.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "binding_conflict"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The receipt's instance, generation, Server boot, or instance deadline no longer matches; returns HTTP 409 and requires fencing before obtaining valid eligibility again.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "receipt_instance_mismatch"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The receipt target is neither current nor a valid historical reference held by this Node; returns HTTP 409 and requires fencing and entering the blocked state.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "receipt_target_mismatch"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The current publication's execution eligibility was withdrawn or changed; returns HTTP 409. Discard the rejected receipt, retry control refresh after one second, and create a new installation attempt on the same instance.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "receipt_publication_superseded"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "An applied receipt for the current enabled target lacks an unexpired execution lease; returns HTTP 409. Discard the rejected receipt, retry control refresh after one second, and create a new installation attempt without extending the old lease.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "receipt_lease_expired"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The receipt observation time is out of bounds or applied lacks an earlier matching saved receipt; returns HTTP 409. Discard the rejected receipt, retry control refresh after one second, and confirm saved again in a new installation attempt.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "receipt_precondition"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A receipt reuses an instance/publication/attempt/sequence identity with different content or has incompatible phase and failure values; returns HTTP 409. Discard it without rewriting the conflicting receipt, retry control refresh after one second, and create a new installation attempt.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "receipt_content_conflict"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The resource name changed since the client read it; returns HTTP 409, so reload before retrying.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "name_conflict"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The pagination cursor is invalid or expired; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_cursor"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The resource version changed since the client read it; returns HTTP 409, so reload before retrying.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "version_conflict"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The operation would remove or demote the network's last Owner; returns HTTP 409.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "last_owner"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The user attempting to join already belongs to the network; returns HTTP 409.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "already_member"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The join token is invalid or no longer usable; returns HTTP 400 without exposing unverified link details.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_join_link"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The verified join token has reached its expiry time; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "join_link_expired"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The verified one-time join token has already been consumed; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "join_link_used"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The verified join token was revoked; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "join_link_revoked"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The verified join token's issuer is no longer authorized to grant membership; returns HTTP 400.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "join_link_issuer_unavailable"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The delegation code is unknown, expired, already exchanged, or does not match its client, redirect target or PKCE verifier; returns HTTP 400 and the client must start a new authorization.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "invalid_delegation"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The expected configuration revision no longer matches the current revision; returns HTTP 409, so re-read the baseline and revalidate the preserved draft before applying again.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "revision_conflict"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Configuration validation fails or required preparation is incomplete; returns HTTP 422 with ConfigurationValidationParams containing issues and requirements.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "configuration_invalid"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/ConfigurationValidationParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The current lifecycle state or sequence does not permit the operation; returns HTTP 409.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "state_conflict"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The State report's execution scope changed; returns HTTP 409, so refresh the scope and observe again.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "state_scope_changed"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The operation exceeds the State resource budget; returns HTTP 429.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "state_limit"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The operation exceeds the observation budget; returns HTTP 429.",
            "required": [
              "code",
              "params",
              "message"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "observation_limit"
                ]
              },
              "message": {
                "type": "string",
                "description": "Safe fallback text. Clients branch on code."
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams",
                "description": "Structured context defined by this problem code, or an empty object when none is exposed."
              }
            },
            "additionalProperties": false
          }
        ]
      },
      "ProfileAssignment": {
        "type": "object",
        "description": "Allowed egress profiles for matching nodes, merged with overlapping assignments.",
        "required": [
          "target",
          "allowed",
          "default"
        ],
        "properties": {
          "allowed": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Permitted named profiles or built-ins `mesh`, `auto` and `global:<exit-node-id>`; overlapping assignments take the union."
          },
          "default": {
            "type": "string",
            "description": "Default profile, required in the merged allowed set and identical across overlapping assignments."
          },
          "locked": {
            "type": "boolean",
            "description": "Prevent user profile changes; defaults to false and must agree across overlapping assignments."
          },
          "target": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Nonempty Mesh selectors: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`."
          }
        },
        "additionalProperties": false
      },
      "ProfileEvaluation": {
        "type": "object",
        "description": "The egress profile the plan evaluated and how the source came to use it.",
        "required": [
          "name",
          "assignment",
          "allowed",
          "locked",
          "requested"
        ],
        "properties": {
          "allowed": {
            "type": "array",
            "items": {
              "type": "string"
            }
          },
          "assignment": {
            "type": [
              "string",
              "null"
            ],
            "description": "The assignment whose default applies, or null for the built-in `mesh`."
          },
          "locked": {
            "type": "boolean"
          },
          "name": {
            "type": "string",
            "description": "Profile name: `mesh`, `auto`, `global:<nodeId>` or a configured profile key."
          },
          "requested": {
            "type": "boolean",
            "description": "Whether the request named this profile (true) or it is the source's\ncurrent selection / default (false)."
          }
        },
        "additionalProperties": false
      },
      "ProgressScope": {
        "type": "string",
        "description": "Whether progress covers the entire original apply target set or a permission-filtered subset.",
        "enum": [
          "full",
          "partial"
        ]
      },
      "ProofOperation": {
        "type": "string",
        "description": "Operation bound into the signed Node challenge transcript.",
        "enum": [
          "session",
          "rotate"
        ]
      },
      "PublicationId": {
        "type": "string",
        "format": "uuid",
        "description": "Execution publication identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "PublicationProgress": {
        "type": "object",
        "description": "Administrator-visible execution publication targets and Server-derived receipt progress.",
        "required": [
          "publication",
          "revision",
          "sequence",
          "withdrawnAt",
          "nodes"
        ],
        "properties": {
          "nodes": {
            "type": "object",
            "description": "Per-Node targets and receipt projections for this publication.",
            "additionalProperties": {
              "$ref": "#/components/schemas/NodePublicationProgress"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "publication": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Execution publication whose fixed targets are described."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Source configuration revision used by the publication."
          },
          "sequence": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Stored sequence of this execution publication."
          },
          "withdrawnAt": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Timestamp",
                "description": "UTC withdrawal time, or null while the publication has not been withdrawn."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "QueryUdpTraversalRequest": {
        "type": "object",
        "description": "Node query for current STUN metadata and its own valid UDP acquisition attempts without renewing them.",
        "required": [
          "instance",
          "configuration"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact current execution target against which acquisition scope is checked."
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current runtime instance making the query."
          }
        },
        "additionalProperties": false
      },
      "QueryUdpTraversalResponse": {
        "oneOf": [
          {
            "type": "object",
            "description": "Current coordination metadata under the caller's checked execution scope.",
            "required": [
              "stateEpoch",
              "asOf",
              "expiresAt",
              "stunServers",
              "attempts",
              "status"
            ],
            "properties": {
              "asOf": {
                "$ref": "#/components/schemas/Timestamp",
                "description": "Server evaluation time in milliseconds since the Unix epoch, serialized as UTC RFC 3339."
              },
              "attempts": {
                "$ref": "#/components/schemas/UdpTraversalAttempts",
                "description": "Valid nonterminal attempts involving the authenticated Node."
              },
              "expiresAt": {
                "$ref": "#/components/schemas/Timestamp",
                "description": "Metadata deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and bounded by current authority and 30 seconds."
              },
              "stateEpoch": {
                "$ref": "#/components/schemas/StateEpoch",
                "description": "State continuity epoch required by subsequent prepare, accept and cancel requests."
              },
              "status": {
                "type": "string",
                "enum": [
                  "current"
                ]
              },
              "stunServers": {
                "$ref": "#/components/schemas/StunServers",
                "description": "Explicitly configured STUN endpoints; an empty list supplies no discovery server."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Current scope or coordination availability prevents returning usable metadata.",
            "required": [
              "reason",
              "status"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/UdpTraversalUnavailableReason",
                "description": "Closed coordination outcome requiring the caller to reevaluate acquisition."
              },
              "status": {
                "type": "string",
                "enum": [
                  "unavailable"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "A current-authority snapshot. Reading it never renews attempts or observations."
      },
      "ReceiptFreshness": {
        "type": "string",
        "description": "Server evaluation of target retirement, instance binding and execution deadlines.",
        "enum": [
          "current",
          "expired",
          "awaitingInstance",
          "superseded"
        ]
      },
      "ReceiptPhase": {
        "type": "string",
        "description": "Durable save or installation outcome reported by the Node for an exact execution target.",
        "enum": [
          "saved",
          "applied",
          "failed"
        ]
      },
      "ReceiptRequest": {
        "type": "object",
        "description": "Node-reported durable result ordered by instance, target, attempt and sequence rather than HTTP arrival.",
        "required": [
          "instance",
          "target",
          "phase",
          "attempt",
          "sequence",
          "reportedAt"
        ],
        "properties": {
          "attempt": {
            "$ref": "#/components/schemas/Version",
            "description": "Increasing application attempt within this instance/target."
          },
          "failure": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ApplyFailure",
                "description": "Failed step required for the failed phase; null or omitted for saved and applied."
              },
              {
                "type": "null"
              }
            ]
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Runtime instance and generation that performed this attempt."
          },
          "phase": {
            "$ref": "#/components/schemas/ReceiptPhase",
            "description": "Actual durable save or installation result."
          },
          "reportedAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Original Node result time in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and preserved on retry."
          },
          "sequence": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Increasing result sequence within the attempt; delivery retries reuse it."
          },
          "target": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact complete target saved or installed by the attempt."
          }
        },
        "additionalProperties": false
      },
      "RecentLink": {
        "type": "object",
        "description": "Recent channel summary derived from stored events, not a claim of current connectivity.",
        "required": [
          "sourceNodeId",
          "peerNodeId",
          "linkId",
          "transport",
          "lastEventAt",
          "closed"
        ],
        "properties": {
          "closed": {
            "type": "boolean",
            "description": "Whether the latest contributing channel event reports closure."
          },
          "lastEventAt": {
            "type": "string",
            "format": "date-time",
            "description": "UTC occurrence time of the latest contributing event."
          },
          "linkId": {
            "type": "string",
            "description": "Actual channel identifier retained from the events."
          },
          "peerNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Adjacent peer identified by the channel events."
          },
          "sourceNodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that reported the channel events."
          },
          "transport": {
            "type": "string",
            "description": "Transport used by the reported channel."
          }
        },
        "additionalProperties": false
      },
      "RedeemEnrollmentRequest": {
        "type": "object",
        "description": "Device redemption of a user-approved enrollment result.",
        "required": [
          "proof",
          "code"
        ],
        "properties": {
          "code": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "One-time approval result delivered through the device's authenticated query."
          },
          "proof": {
            "$ref": "#/components/schemas/EnrollmentDeviceProof",
            "description": "Device signature and PKCE verifier proving ownership of the enrollment request."
          }
        },
        "additionalProperties": false
      },
      "RefreshRuleSetRequest": {
        "type": "object",
        "description": "Idempotent manual refresh of one remote rule set.",
        "required": [
          "requestId"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Key retained for 24 hours per user and network; same-key retries return the current view, while reuse for another key conflicts."
          }
        },
        "additionalProperties": false
      },
      "RegisterRequest": {
        "type": "object",
        "description": "Credentials for creating a new account without granting network privileges.",
        "required": [
          "email",
          "password"
        ],
        "properties": {
          "email": {
            "type": "string",
            "description": "Login address, normalized by trimming, case folding, and IDNA domain conversion."
          },
          "password": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "New password satisfying the advertised password rules, without trimming."
          }
        },
        "additionalProperties": false
      },
      "RemovalImpactResponse": {
        "type": "object",
        "description": "Current User-identity nodes that would be permanently deleted by membership removal.",
        "required": [
          "requestId",
          "membershipId",
          "version",
          "revision",
          "nodeIds",
          "nodeCount"
        ],
        "properties": {
          "membershipId": {
            "$ref": "#/components/schemas/MembershipId",
            "description": "Membership whose removal is being previewed."
          },
          "nodeCount": {
            "type": "integer",
            "format": "int32",
            "description": "Number of nodes in the removal set.",
            "minimum": 0
          },
          "nodeIds": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Complete set of nodes currently bound to this User membership, excluding Tag nodes."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "revision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Configuration revision used to compute the removal impact."
          },
          "version": {
            "$ref": "#/components/schemas/Version",
            "description": "Current membership version to use as the removal concurrency guard."
          }
        },
        "additionalProperties": false
      },
      "RemoveMemberRequest": {
        "type": "object",
        "description": "Member removal or network departure guarded by membership and configuration versions.",
        "required": [
          "expectedVersion",
          "expectedRevision"
        ],
        "properties": {
          "expectedRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Configuration revision read when reviewing the affected User-identity nodes."
          },
          "expectedVersion": {
            "$ref": "#/components/schemas/Version",
            "description": "Membership version read when reviewing the removal."
          }
        },
        "additionalProperties": false
      },
      "RenameNetworkRequest": {
        "type": "object",
        "description": "Network rename guarded by the previously read name and a request idempotency key.",
        "required": [
          "requestId",
          "name",
          "expectedName"
        ],
        "properties": {
          "expectedName": {
            "type": "string",
            "description": "Previously read network name that must still match when the rename is committed."
          },
          "name": {
            "type": "string",
            "description": "Raw input. Apply getAuthOptions.networkNameRules, not String.length."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Caller-chosen idempotency key bound to the caller, network, and rename input."
          }
        },
        "additionalProperties": false
      },
      "RenewDemandRequest": {
        "type": "object",
        "description": "Explicit renewal of a live demand without changing its original business intent.",
        "required": [
          "instance",
          "requestId",
          "expiresAt"
        ],
        "properties": {
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Absolute desired deadline of the existing demand; replay never adds duration."
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current source instance that owns the existing demand."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Existing demand identifier, not a new renewal request identifier."
          }
        },
        "additionalProperties": false
      },
      "RenewNodeObservation": {
        "type": "object",
        "description": "Explicit renewal of the caller's existing Node observation subscription under current permissions.",
        "required": [
          "instance",
          "expiresAt"
        ],
        "properties": {
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Requested absolute subscription deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and subject to lease bounds."
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current source instance that owns the subscription."
          }
        },
        "additionalProperties": false
      },
      "ReplyPolicy": {
        "type": "string",
        "description": "Reply-path constraint attached to a compiled business permit.",
        "enum": [
          "statefulExactReversePath"
        ]
      },
      "ReportRef": {
        "type": "object",
        "description": "Exact accepted Node snapshot contributing an aggregate fact.",
        "required": [
          "nodeId",
          "instanceId",
          "reportSequence"
        ],
        "properties": {
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Runtime instance that produced the report."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that produced the original report."
          },
          "reportSequence": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Sequence of the contributing report within that instance."
          }
        },
        "additionalProperties": false
      },
      "ReportedObservationPhase": {
        "type": "string",
        "description": "Lifecycle phases a Node may report without supplying a measurement sample.",
        "enum": [
          "preparing",
          "failed",
          "completed",
          "unsupported"
        ]
      },
      "ReporterRole": {
        "type": "string",
        "description": "Reporting Node's role in the complete path it observed.",
        "enum": [
          "origin",
          "relay",
          "target",
          "exit"
        ]
      },
      "RequestId": {
        "type": "string",
        "format": "uuid",
        "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "ResetReason": {
        "type": "string",
        "description": "Reason a stream must replace its cached view through full synchronization.",
        "enum": [
          "cursor_too_old",
          "view_changed",
          "backpressure",
          "epoch_changed"
        ]
      },
      "ResourceProblem": {
        "type": "string",
        "description": "Stable failure category for rule-set preparation or refresh.",
        "enum": [
          "download_failed",
          "invalid_source",
          "too_large",
          "conversion_failed",
          "configuration_changed",
          "budget_exceeded",
          "not_ready",
          "validation_failed"
        ]
      },
      "ResourceRef": {
        "oneOf": [
          {
            "type": "object",
            "description": "A configured inbound listener resource.",
            "required": [
              "key",
              "kind"
            ],
            "properties": {
              "key": {
                "type": "string",
                "description": "Listener key in the associated configuration."
              },
              "kind": {
                "type": "string",
                "enum": [
                  "listener"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "An outbound dialing resource.",
            "required": [
              "key",
              "kind"
            ],
            "properties": {
              "key": {
                "type": "string",
                "description": "Dial resource key in the associated execution configuration."
              },
              "kind": {
                "type": "string",
                "enum": [
                  "dial"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A coordinated connection acquisition resource.",
            "required": [
              "key",
              "kind"
            ],
            "properties": {
              "key": {
                "type": "string",
                "description": "Coordination resource key in the associated execution configuration."
              },
              "kind": {
                "type": "string",
                "enum": [
                  "coordination"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Exact configured or execution resource associated with a runtime socket."
      },
      "Revision": {
        "type": "integer",
        "description": "A positive configuration revision.",
        "maximum": 9007199254740991,
        "minimum": 1
      },
      "RevokeNodeKeyRequest": {
        "type": "object",
        "description": "Permanent revocation of an exact Node key binding.",
        "required": [
          "keyId",
          "expectedGeneration"
        ],
        "properties": {
          "expectedGeneration": {
            "$ref": "#/components/schemas/Version",
            "description": "Key generation required to prevent revoking an unexpected replacement."
          },
          "keyId": {
            "$ref": "#/components/schemas/KeyId",
            "description": "Bound control or data-plane key to revoke."
          }
        },
        "additionalProperties": false
      },
      "RoleChangeImpactResponse": {
        "type": "object",
        "description": "Live join links that would be revoked by the proposed membership role change.",
        "required": [
          "requestId",
          "membershipId",
          "version",
          "role",
          "joinLinks",
          "joinLinkCount"
        ],
        "properties": {
          "joinLinkCount": {
            "type": "integer",
            "format": "int32",
            "description": "Total number of affected links, including those beyond the returned list.",
            "minimum": 0
          },
          "joinLinks": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/JoinLinkView"
            },
            "description": "At most 100 affected unused, unexpired join links, newest first."
          },
          "membershipId": {
            "$ref": "#/components/schemas/MembershipId",
            "description": "Membership whose role change is being previewed."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated correlation ID for this HTTP request."
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole",
            "description": "Proposed role used to calculate the impact."
          },
          "version": {
            "$ref": "#/components/schemas/Version",
            "description": "Current membership version to use as the role-change concurrency guard."
          }
        },
        "additionalProperties": false
      },
      "RouteAction": {
        "oneOf": [
          {
            "type": "object",
            "description": "Forward through an authorized exit along an allowed Mesh path.",
            "required": [
              "nodes",
              "type"
            ],
            "properties": {
              "nodes": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Nonempty ordered primary and backup `node:<id>` exits with `exitEnabled` set."
              },
              "type": {
                "type": "string",
                "enum": [
                  "exit"
                ]
              },
              "via": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Ordered required `node:<id>` relays before the exit; defaults to an empty list."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Send traffic directly through the source node's local network.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "direct"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Leave unmatched traffic to the local system without Mesh interception.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "unmanaged"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Fallback action for destinations not matched by public or selected-profile rules."
      },
      "RouteEvaluation": {
        "type": "object",
        "description": "Where traffic to the destination goes and which configuration element decided.",
        "required": [
          "kind",
          "rule",
          "profile",
          "ruleSet",
          "targets",
          "via",
          "resolutionDependent"
        ],
        "properties": {
          "kind": {
            "$ref": "#/components/schemas/RouteKind",
            "description": "`mesh` for a Mesh destination, otherwise the matched rule's action or the profile fallback."
          },
          "profile": {
            "type": [
              "string",
              "null"
            ],
            "description": "The profile whose rules decided, after `auto` resolution; null for a\npublic rule or a Mesh destination."
          },
          "resolutionDependent": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Earlier rules that were not judged because they match the other kind of\ndestination (by address for a domain, by name for an address)."
          },
          "rule": {
            "type": [
              "string",
              "null"
            ],
            "description": "The deciding element: an egress rule, a profile `fallback`, or a Mesh\nroute. Null when implicit: automatic Mesh routing, the built-in `mesh`\nprofile leaving external traffic unmanaged, or a `global:` profile."
          },
          "ruleSet": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/RuleSetMatch",
                "description": "The rule-set entry through which the rule matched, when it did."
              },
              {
                "type": "null"
              }
            ]
          },
          "targets": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Ordered exit candidates of an exit decision; the Mesh destination itself otherwise."
          },
          "via": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Mandatory waypoints: the exit rule's `via`, or the Mesh route's `via` for a Mesh destination."
          }
        },
        "additionalProperties": false
      },
      "RouteKind": {
        "type": "string",
        "description": "The action a route decision takes.",
        "enum": [
          "mesh",
          "exit",
          "direct",
          "unmanaged"
        ]
      },
      "RouteMatch": {
        "type": "object",
        "description": "Union of destination conditions for an egress rule; at least one condition list must be nonempty.",
        "properties": {
          "domain": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Exact DNS names, excluding subdomains; defaults to an empty list and is omitted from canonical output when empty."
          },
          "domainSuffix": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "DNS label-boundary suffixes; `example.org` matches itself and `a.example.org`, and the list defaults to empty."
          },
          "ipCIDR": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Canonical destination IPv4 or IPv6 CIDRs; defaults to an empty list."
          },
          "ruleSets": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "Referenced rule-set keys whose destinations join this match; defaults to an empty list."
          }
        },
        "additionalProperties": false
      },
      "RoutingConfiguration": {
        "type": "object",
        "description": "Mesh path requirements and external destination routing.",
        "properties": {
          "egress": {
            "$ref": "#/components/schemas/EgressRouting",
            "description": "External destination rules and profiles; omission supplies empty rules, profiles and assignments."
          },
          "mesh": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MeshRoute"
            },
            "description": "Ordered recursive Mesh path rules; an empty list uses automatic routing over allowed topology."
          }
        },
        "additionalProperties": false
      },
      "RoutingProfile": {
        "type": "object",
        "description": "Named egress plan evaluated after network-wide public rules.",
        "required": [
          "fallback"
        ],
        "properties": {
          "fallback": {
            "$ref": "#/components/schemas/RouteAction",
            "description": "Action for destinations unmatched by public and profile rules."
          },
          "rules": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/EgressRule"
            },
            "description": "First-match destination rules in author order; defaults to an empty list."
          }
        },
        "additionalProperties": false
      },
      "RoutingSelectionRequest": {
        "type": "object",
        "description": "Node request to select an authorized routing profile using compare-and-swap against its current selection.",
        "required": [
          "instance",
          "profile"
        ],
        "properties": {
          "expectedVersion": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/Version",
                "description": "Existing selection version to replace; null or omission requires that no selection record exists."
              },
              {
                "type": "null"
              }
            ]
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current runtime instance authorized to change the selection."
          },
          "profile": {
            "type": "string",
            "description": "Allowed routing profile to select for subsequent demands and DNS execution."
          }
        },
        "additionalProperties": false
      },
      "RoutingSelectionResponse": {
        "type": "object",
        "description": "Server-committed routing selection and the execution publication produced by the change.",
        "required": [
          "version",
          "profile",
          "publication"
        ],
        "properties": {
          "profile": {
            "type": "string",
            "description": "Profile selected by the accepted request."
          },
          "publication": {
            "$ref": "#/components/schemas/PublicationId",
            "description": "Execution publication produced after cancelling the source's prior demands."
          },
          "version": {
            "$ref": "#/components/schemas/Version",
            "description": "New selection version required by the next compare-and-swap update."
          }
        },
        "additionalProperties": false
      },
      "RuleAction": {
        "oneOf": [
          {
            "type": "object",
            "description": "Forward through an authorized exit along an allowed Mesh path.",
            "required": [
              "nodes",
              "type"
            ],
            "properties": {
              "nodes": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Nonempty ordered primary and backup `node:<id>` exits with `exitEnabled` set."
              },
              "type": {
                "type": "string",
                "enum": [
                  "exit"
                ]
              },
              "via": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Ordered required `node:<id>` relays before the exit; defaults to an empty list."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Send matching traffic directly through the source node's local network.",
            "required": [
              "type"
            ],
            "properties": {
              "type": {
                "type": "string",
                "enum": [
                  "direct"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Action selected by the first matching egress rule."
      },
      "RuleSet": {
        "oneOf": [
          {
            "type": "object",
            "description": "Define destinations directly, with at least one nonempty condition list.",
            "required": [
              "type"
            ],
            "properties": {
              "domain": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Exact DNS names without subdomains; defaults to empty and is omitted from canonical output when empty."
              },
              "domainSuffix": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "DNS label-boundary suffixes matching the name itself and its subdomains; defaults to an empty list."
              },
              "ipCIDR": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Canonical IPv4 or IPv6 destination CIDRs; defaults to an empty list."
              },
              "type": {
                "type": "string",
                "enum": [
                  "inline"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Fetch a supported source and bind its immutable normalized content to execution publications.",
            "required": [
              "url",
              "format",
              "type"
            ],
            "properties": {
              "format": {
                "$ref": "#/components/schemas/RuleSetFormat",
                "description": "Source format to convert without dropping unsupported entries."
              },
              "type": {
                "type": "string",
                "enum": [
                  "remote"
                ]
              },
              "updateInterval": {
                "$ref": "#/components/schemas/PositiveDuration",
                "description": "Positive integer duration with `ms`, `s`, `m` or `h`; defaults to `24h`, with scheduling bounded below by one second."
              },
              "url": {
                "type": "string",
                "description": "HTTPS source URL without userinfo or fragment; downloads reject redirects and nonpublic resolved addresses."
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Destination set declared inline or downloaded and validated by the Server."
      },
      "RuleSetFormat": {
        "type": "string",
        "description": "Remote source format whose supported subset converts exactly to IP and DNS destination sets.",
        "enum": [
          "sing-box-source",
          "mihomo-domain",
          "mihomo-ipcidr"
        ]
      },
      "RuleSetList": {
        "type": "object",
        "description": "Authorized resource views for all rule sets or one requested key.",
        "required": [
          "items"
        ],
        "properties": {
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/RuleSetView"
            },
            "description": "Current definitions and check results, restricted to the selected key for a detail request."
          }
        },
        "additionalProperties": false
      },
      "RuleSetMatch": {
        "type": "object",
        "description": "The rule-set entry through which a route rule matched.",
        "required": [
          "key",
          "entry"
        ],
        "properties": {
          "entry": {
            "type": "string",
            "description": "The matching entry: a CIDR, a suffix or an exact name, after normalization."
          },
          "key": {
            "type": "string",
            "description": "Rule set key as referenced by the rule's `ruleSets`."
          }
        },
        "additionalProperties": false
      },
      "RuleSetView": {
        "type": "object",
        "description": "Current rule-set declaration, selected immutable artifact and actual check status.",
        "required": [
          "key",
          "configurationRevision"
        ],
        "properties": {
          "artifact": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ArtifactSummary",
                "description": "Selected validated artifact, retained after failed refreshes; null when no artifact is bound, including inline definitions."
              },
              {
                "type": "null"
              }
            ]
          },
          "checkedAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "UTC time of the latest persisted check, successful or failed; null when no check record exists."
          },
          "configurationRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Current source revision owning this definition and resource view."
          },
          "definition": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/RuleSet",
                "description": "Source declaration when included in the authorized resource scope; null when the scope does not expose it."
              },
              {
                "type": "null"
              }
            ]
          },
          "key": {
            "type": "string",
            "description": "Rule-set key in the network configuration."
          },
          "lastSuccessAt": {
            "type": [
              "string",
              "null"
            ],
            "format": "date-time",
            "description": "Last successful download/conversion/publication check, not artifact creation."
          },
          "problem": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ResourceProblem",
                "description": "Last check failure or `not_ready` for missing initial remote content; null when no problem is recorded."
              },
              {
                "type": "null"
              }
            ]
          },
          "publication": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/PublicationId",
                "description": "Exact execution publication whose immutable bindings select this artifact."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "SecretDelivery": {
        "type": "string",
        "description": "Availability of a creation secret in this response.",
        "enum": [
          "available",
          "unavailable"
        ]
      },
      "SecretString": {
        "type": "string",
        "description": "Serialization is secret-bearing and requires protected storage."
      },
      "SelectionState": {
        "type": "object",
        "description": "Node-reported routing profile currently in use.",
        "properties": {
          "appliedProfile": {
            "type": [
              "string",
              "null"
            ],
            "description": "Actually applied profile within configuration constraints, or null when none is reported."
          }
        },
        "additionalProperties": false
      },
      "Sequence": {
        "type": "integer",
        "description": "A nonnegative sequence number.",
        "maximum": 9007199254740991,
        "minimum": 0
      },
      "SessionId": {
        "type": "string",
        "format": "uuid",
        "description": "User session record identifier; not the session token.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "SessionIssuedResponse": {
        "type": "object",
        "description": "First delivery only. Session reads never return the token.",
        "required": [
          "requestId",
          "user",
          "session",
          "token"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Server-generated request correlation ID, not a registration idempotency key."
          },
          "session": {
            "$ref": "#/components/schemas/SessionMetadata",
            "description": "Identifier and validity period of the issued session."
          },
          "token": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "Opaque bearer credential delivered only in this issuance response."
          },
          "user": {
            "$ref": "#/components/schemas/UserView",
            "description": "Account authenticated by the issued session."
          }
        },
        "additionalProperties": false
      },
      "SessionMetadata": {
        "type": "object",
        "description": "Non-secret metadata for an opaque user session.",
        "required": [
          "id",
          "createdAt",
          "expiresAt"
        ],
        "properties": {
          "createdAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the session was issued."
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time",
            "description": "Time at which the session becomes invalid, even if it has been used recently."
          },
          "id": {
            "$ref": "#/components/schemas/SessionId",
            "description": "Stable identifier of this user session."
          }
        },
        "additionalProperties": false
      },
      "SignedNodeChallengeRequest": {
        "type": "object",
        "description": "Proof of the current control key authorizing challenge allocation before a Node session exists.",
        "required": [
          "intent",
          "signature"
        ],
        "properties": {
          "intent": {
            "$ref": "#/components/schemas/NodeChallengeIntent",
            "description": "Exact application covered by the separate challenge-request signature transcript."
          },
          "signature": {
            "$ref": "#/components/schemas/WireSecret",
            "description": "64-byte Ed25519 signature encoded in unpadded base64url."
          }
        },
        "additionalProperties": false
      },
      "SocketAddress": {
        "type": "string",
        "format": "meshway-socket-address",
        "description": "A numeric socket address with a nonzero port and no IPv6 scope or flow info.\nThis observed fact alone does not establish reachability or grant authority."
      },
      "SocketNetwork": {
        "type": "string",
        "description": "Actual outer socket protocol, independent of the inner business protocol.",
        "enum": [
          "udp",
          "tcp"
        ]
      },
      "SocketOptions": {
        "type": "object",
        "description": "Requested socket buffer sizes, subject to provider defaults and platform resource limits.",
        "properties": {
          "receiveBuffer": {
            "type": "integer",
            "format": "int32",
            "description": "Receive buffer size in bytes; omitted or zero uses the provider default.",
            "minimum": 0
          },
          "sendBuffer": {
            "type": "integer",
            "format": "int32",
            "description": "Send buffer size in bytes; omitted or zero uses the provider default, and the kernel may limit the requested size.",
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "SocketReference": {
        "type": "object",
        "description": "Exact Node-reported socket binding checked against the current instance and execution target.",
        "required": [
          "nodeId",
          "instance",
          "configuration",
          "socketId",
          "socketGeneration",
          "resource"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Exact installed execution target associated with the socket."
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Owning runtime instance and its Server-issued generation."
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node that owns and reports the actual socket."
          },
          "resource": {
            "$ref": "#/components/schemas/ResourceRef",
            "description": "Configured resource for which this socket was created."
          },
          "socketGeneration": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Socket incarnation within the runtime instance, preventing reuse of stale references."
          },
          "socketId": {
            "type": "string",
            "description": "Node-assigned socket identifier of 1–256 printable non-space ASCII bytes.",
            "maxLength": 256,
            "minLength": 1,
            "pattern": "^[!-~]{1,256}(?![\\s\\S])"
          }
        },
        "additionalProperties": false
      },
      "SocketState": {
        "type": "object",
        "description": "Actual outer socket reported by the Node and indexed by its instance-local socket ID.",
        "required": [
          "resource",
          "generation",
          "network",
          "boundAddress",
          "endpoints"
        ],
        "properties": {
          "boundAddress": {
            "type": "string",
            "description": "Address and port actually bound by the operating system."
          },
          "configuration": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "None inherits the report's applied configuration; installation sockets\nmust name their target even before the first configuration is applied."
              },
              {
                "type": "null"
              }
            ]
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/EndpointCandidate"
            },
            "description": "Candidates associated with this socket, retaining their original observation deadlines."
          },
          "generation": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Resource generation that distinguishes replacement sockets using the same address."
          },
          "localEndpoints": {
            "type": [
              "array",
              "null"
            ],
            "items": {
              "$ref": "#/components/schemas/LocalEndpoint"
            },
            "description": "Omitted when not sampled; empty means enumeration found no usable address.",
            "maxItems": 8
          },
          "network": {
            "$ref": "#/components/schemas/SocketNetwork",
            "description": "Outer protocol used by the socket."
          },
          "resource": {
            "$ref": "#/components/schemas/ResourceRef",
            "description": "Exact resource owning this socket."
          }
        },
        "additionalProperties": false
      },
      "StateAccepted": {
        "type": "object",
        "description": "Acceptance of a State report, not an execution lease or application receipt.",
        "required": [
          "sequence",
          "receivedAt",
          "duplicate"
        ],
        "properties": {
          "duplicate": {
            "type": "boolean",
            "description": "Whether the same sequence and content had already been accepted."
          },
          "receivedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original Server reception time, preserved for identical retries."
          },
          "sequence": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Accepted report sequence."
          }
        },
        "additionalProperties": false
      },
      "StateEpoch": {
        "type": "string",
        "format": "uuid",
        "description": "Volatile controller state epoch.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "StateProblem": {
        "oneOf": [
          {
            "type": "object",
            "description": "A required runtime resource is unavailable.",
            "required": [
              "resource",
              "observedAt",
              "code"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "resource_unavailable"
                ]
              },
              "observedAt": {
                "type": "string",
                "format": "date-time",
                "description": "UTC time at which the Node observed the failure."
              },
              "resource": {
                "$ref": "#/components/schemas/ResourceRef",
                "description": "Resource that the Node could not use."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The Node rejected an execution target.",
            "required": [
              "target",
              "observedAt",
              "code"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "configuration_rejected"
                ]
              },
              "observedAt": {
                "type": "string",
                "format": "date-time",
                "description": "UTC time at which the Node observed the rejection."
              },
              "target": {
                "$ref": "#/components/schemas/ConfigurationRef",
                "description": "Exact target rejected by the Node."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A specific measurement failed.",
            "required": [
              "measurementId",
              "observedAt",
              "code"
            ],
            "properties": {
              "code": {
                "type": "string",
                "enum": [
                  "probe_failed"
                ]
              },
              "measurementId": {
                "$ref": "#/components/schemas/EventId",
                "description": "Identifier of the failed measurement."
              },
              "observedAt": {
                "type": "string",
                "format": "date-time",
                "description": "UTC time at which the Node observed the failure."
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Typed Node-reported failure retaining the affected resource or measurement."
      },
      "StateQuery": {
        "type": "object",
        "description": "Optional subset filter that can only narrow the caller's authorized State view.",
        "properties": {
          "nodes": {
            "type": [
              "array",
              "null"
            ],
            "items": {
              "$ref": "#/components/schemas/NodeId"
            },
            "description": "Requested subset; null or omission uses all visible Nodes, while an empty list returns none."
          }
        },
        "additionalProperties": false
      },
      "StateReport": {
        "type": "object",
        "description": "Complete Node snapshot authenticated and validated against the current instance.",
        "required": [
          "instanceId",
          "sequence",
          "observedAt",
          "state"
        ],
        "properties": {
          "instanceId": {
            "$ref": "#/components/schemas/InstanceId",
            "description": "Current authenticated runtime instance that produced the snapshot."
          },
          "observedAt": {
            "type": "string",
            "format": "date-time",
            "description": "Original UTC time at which the Node observed the snapshot."
          },
          "sequence": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Increasing report sequence; identical retries reuse the sequence and content."
          },
          "state": {
            "$ref": "#/components/schemas/NodeState",
            "description": "Complete runtime inventory replacing the previous accepted snapshot."
          }
        },
        "additionalProperties": false
      },
      "StateView": {
        "type": "object",
        "description": "Permission-filtered network snapshot assembled from independently timed Node reports.",
        "required": [
          "networkId",
          "stateEpoch",
          "sequence",
          "configurationRevision",
          "asOf",
          "nodes",
          "links",
          "paths"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time",
            "description": "UTC time at which the Server evaluated this view."
          },
          "configurationRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Current configuration used to interpret and filter the view."
          },
          "links": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/AggregatedLink"
            },
            "description": "Channel evidence aggregated from visible reports."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network whose visible State is returned."
          },
          "nodes": {
            "type": "object",
            "description": "Visible Node reports indexed by Node identity.",
            "additionalProperties": {
              "$ref": "#/components/schemas/NodeStateView"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "paths": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/AggregatedPath"
            },
            "description": "Path observations retaining each original reporter's evidence."
          },
          "sequence": {
            "$ref": "#/components/schemas/Sequence",
            "description": "State change sequence within the Server epoch."
          },
          "stateEpoch": {
            "$ref": "#/components/schemas/StateEpoch",
            "description": "Server State generation, replaced after a Server restart."
          }
        },
        "additionalProperties": false
      },
      "StreamKind": {
        "type": "string",
        "description": "Outer stream encapsulation supported by a transport implementation.",
        "enum": [
          "tcp",
          "websocket",
          "udp"
        ]
      },
      "StunServers": {
        "type": "array",
        "items": {
          "type": "object",
          "description": "Explicit STUN discovery endpoint published by the Server operator.",
          "required": [
            "id",
            "endpoint"
          ],
          "properties": {
            "endpoint": {
              "$ref": "#/components/schemas/Endpoint",
              "description": "Explicit operator metadata; a hostname is allowed here, not in probe targets."
            },
            "id": {
              "type": "string",
              "description": "Operator metadata identifier containing 1–64 ASCII letters, digits, underscores or hyphens.",
              "maxLength": 64,
              "minLength": 1,
              "pattern": "^[A-Za-z0-9_-]{1,64}(?![\\s\\S])"
            }
          },
          "additionalProperties": false
        },
        "maxItems": 4
      },
      "SystemState": {
        "type": "object",
        "description": "Operating system and build information reported by the Node.",
        "required": [
          "hostname",
          "os",
          "arch",
          "agentVersion"
        ],
        "properties": {
          "agentVersion": {
            "type": "string",
            "description": "Version of the running Node software."
          },
          "arch": {
            "type": "string",
            "description": "Processor architecture of the running Node."
          },
          "hostname": {
            "type": "string",
            "description": "Hostname reported by the operating system, independent of the configured hostname."
          },
          "os": {
            "type": "string",
            "description": "Operating system running the Node."
          }
        },
        "additionalProperties": false
      },
      "TargetEvaluation": {
        "type": "object",
        "description": "One exit candidate or Mesh target and whether a path to it exists.",
        "required": [
          "nodeId",
          "problem",
          "paths"
        ],
        "properties": {
          "exitUnavailableReason": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ExitUnavailableReason",
                "description": "Fresh typed exit failure when reported; absence is unknown, not success."
              },
              {
                "type": "null"
              }
            ]
          },
          "nodeId": {
            "$ref": "#/components/schemas/NodeId"
          },
          "paths": {
            "type": "integer",
            "format": "int32",
            "description": "Number of planned paths to this target, at most four.",
            "minimum": 0
          },
          "problem": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/TargetProblem",
                "description": "Why no path to this target was planned; null when `paths` is nonzero."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "TargetProblem": {
        "type": "string",
        "description": "Why a target has no planned path.",
        "enum": [
          "disabled",
          "exitDisabled",
          "exitUnavailable",
          "unsupportedFamily",
          "notEnrolled",
          "noPath",
          "routingCycle",
          "routingLimit"
        ]
      },
      "TcpKeepAlive": {
        "type": "object",
        "description": "Operating-system TCP keepalive settings, independent of Meshway authorization and lease checks.",
        "properties": {
          "count": {
            "type": "integer",
            "format": "int32",
            "description": "Number of keepalive probes; must be greater than zero and defaults to 3.",
            "minimum": 0
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether to enable operating-system TCP keepalive; defaults to true."
          },
          "idle": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive idle duration before the first keepalive probe; defaults to `5m`."
          },
          "interval": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive duration between keepalive probes; defaults to `75s`."
          }
        },
        "additionalProperties": false
      },
      "TcpOptions": {
        "type": "object",
        "description": "Socket settings shared by carriers with an outer TCP connection.",
        "properties": {
          "keepAlive": {
            "$ref": "#/components/schemas/TcpKeepAlive",
            "description": "Operating-system keepalive settings; omission enables probes after `5m`, every `75s`, with a count of 3."
          },
          "noDelay": {
            "type": "boolean",
            "description": "Whether to enable TCP_NODELAY; defaults to true."
          }
        },
        "additionalProperties": false
      },
      "Timestamp": {
        "type": "string",
        "format": "meshway-timestamp-ms",
        "description": "Nonnegative Unix time as an RFC 3339 string with exact millisecond precision and UTC year at most 9999. Offsets and lowercase t/z are accepted; fractional digits after milliseconds must all be zero. Leap seconds are rejected because they have no lossless Unix-millisecond representation."
      },
      "TlsMode": {
        "type": "string",
        "description": "Source of listener TLS certificate and private-key material; defaults to `managed`.",
        "enum": [
          "managed",
          "provided"
        ]
      },
      "TlsTrust": {
        "type": "string",
        "description": "TLS trust selection for a dialer; only `managed` is accepted.",
        "enum": [
          "managed"
        ]
      },
      "TlsVersion": {
        "type": "string",
        "description": "Supported TLS protocol version bound, encoded as `1.2` or `1.3`.",
        "enum": [
          "1.2",
          "1.3"
        ]
      },
      "TopologyAction": {
        "type": "string",
        "description": "Permission applied to a matching undirected node pair.",
        "enum": [
          "allow",
          "block"
        ]
      },
      "TopologyDefault": {
        "type": "string",
        "description": "Adjacency permission when neither a block nor an allow topology rule matches.",
        "enum": [
          "allow",
          "deny"
        ]
      },
      "TopologyRule": {
        "type": "object",
        "description": "Permission for an undirected adjacency; any matching block precedes the first matching allow.",
        "required": [
          "action",
          "a",
          "b"
        ],
        "properties": {
          "a": {
            "type": "string",
            "description": "One endpoint selector: `*`, user email, `group:<name>`, `tag:<name>` or `node:<id>`."
          },
          "action": {
            "$ref": "#/components/schemas/TopologyAction",
            "description": "Allow adjacency or block the entire edge regardless of other allow rules."
          },
          "b": {
            "type": "string",
            "description": "The other endpoint selector, with the same syntax and no directional distinction from `a`."
          },
          "remark": {
            "type": "string",
            "description": "Free-form description, defaulting to an empty string; duplicate descriptions are allowed."
          },
          "transports": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TransportProtocol"
            },
            "description": "Allowed carriers for this undirected edge; omitted means unrestricted."
          }
        },
        "additionalProperties": false
      },
      "TrafficWindow": {
        "type": "object",
        "description": "Source-scoped business byte counters for a real measurement window; traffic is currently unsupported.",
        "required": [
          "txBytes",
          "rxBytes",
          "counterGeneration"
        ],
        "properties": {
          "counterGeneration": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Counter generation that changes on reset so old delta baselines cannot be reused."
          },
          "rxBytes": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Mesh IP business bytes verified on the return path and delivered locally at the source."
          },
          "txBytes": {
            "$ref": "#/components/schemas/Sequence",
            "description": "Mesh IP business bytes accepted at the source and submitted to the path, excluding control and probe traffic."
          }
        },
        "additionalProperties": false
      },
      "TransportCatalogEntry": {
        "type": "object",
        "description": "Server-supported transport declaration and its required node runtime capability.",
        "required": [
          "protocol",
          "outerSocket",
          "listenerSchema",
          "dialerSchema",
          "requiredCapability"
        ],
        "properties": {
          "dialerSchema": {
            "type": "string",
            "description": "Definition name in getConfigurationSchema.schemaJson.$defs (not schema text)."
          },
          "listenerSchema": {
            "type": "string",
            "description": "Definition name in getConfigurationSchema.schemaJson.$defs (not schema text)."
          },
          "outerSocket": {
            "$ref": "#/components/schemas/OuterSocket",
            "description": "Underlay socket family required by this transport, independent of the inner business protocol."
          },
          "protocol": {
            "$ref": "#/components/schemas/TransportProtocol",
            "description": "Protocol wire name used by listener types, dialer keys and topology restrictions."
          },
          "requiredCapability": {
            "type": "string",
            "description": "Controller configuration support does not attest an Agent build's support."
          }
        },
        "additionalProperties": false
      },
      "TransportCatalogResponse": {
        "type": "object",
        "description": "Supported transport declaration catalog, without asserting runtime availability on a node.",
        "required": [
          "requestId",
          "items"
        ],
        "properties": {
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/TransportCatalogEntry"
            },
            "description": "Available protocol declarations and their schema definition names."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Correlation identifier for this HTTP request."
          }
        },
        "additionalProperties": false
      },
      "TransportProtocol": {
        "type": "string",
        "description": "Protocol keys, not a Controller enabled catalogue or a Node support report.",
        "enum": [
          "native_udp",
          "native_tcp",
          "vless",
          "trojan"
        ]
      },
      "TransportRole": {
        "type": "string",
        "description": "Direction of transport establishment supported by the Node build.",
        "enum": [
          "dial",
          "listen"
        ]
      },
      "TrojanDialer": {
        "type": "object",
        "description": "Outbound carrier selection and connection options; the Server resolves target listener parameters separately.",
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "connectTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive total connection budget including resolution and required handshakes; defaults to `5s` and is limited by the remaining active lease."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether this dialer may be used for outgoing connections; defaults to true."
          },
          "localAddress": {
            "type": "string",
            "description": "Local outgoing IP address; omitted or empty selects automatically, and a Mesh address cannot bypass underlay protection."
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "description": "Preference from 0–65535, lower first; defaults to 100 for native_udp, 200 for native_tcp, 300 for vless, or 400 for trojan.",
            "minimum": 0
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text rather than a resource key; defaults to an empty string."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
          },
          "tls": {
            "$ref": "#/components/schemas/DialerTls",
            "description": "TLS client options; omission uses managed trust, TLS 1.2–1.3, empty ALPN, and a `5s` handshake budget."
          }
        },
        "additionalProperties": false
      },
      "TrojanListener": {
        "type": "object",
        "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
        "required": [
          "listenPort",
          "tls"
        ],
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/Endpoint"
            },
            "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
          },
          "listen": {
            "type": "string",
            "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
          },
          "listenPort": {
            "$ref": "#/components/schemas/Port",
            "description": "Required local listening port in the range 1–65535."
          },
          "password": {
            "$ref": "#/components/schemas/SecretString",
            "description": "Nonempty password preserved without trimming or case normalization; omission makes the Server generate or retain saved material, while null is rejected."
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text; defaults to an empty string and cannot be null."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          },
          "stream": {
            "$ref": "#/components/schemas/TrojanStream",
            "description": "Trojan encapsulation; only `tcp` is accepted and omission defaults to it."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
          },
          "tls": {
            "$ref": "#/components/schemas/ListenerTls",
            "description": "Required TLS server settings, including the certificate verification name and material source."
          }
        },
        "additionalProperties": false
      },
      "TrojanStream": {
        "type": "string",
        "description": "Trojan stream encapsulation; only `tcp` is accepted and is the default.",
        "enum": [
          "tcp"
        ]
      },
      "TrustedDnsAssociation": {
        "type": "object",
        "description": "Trusted source-scoped DNS input used by compilation to bind a domain to destination addresses.",
        "required": [
          "networkId",
          "configurationRevision",
          "source",
          "name",
          "cnameChain",
          "addresses",
          "expiresAt"
        ],
        "properties": {
          "addresses": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/ExecutionAddress"
            },
            "description": "Destination addresses supported by this resolution result."
          },
          "cnameChain": {
            "type": "array",
            "items": {
              "type": "string"
            },
            "description": "CNAME chain retained with the trusted resolution result."
          },
          "configurationRevision": {
            "$ref": "#/components/schemas/Revision",
            "description": "Exact source configuration revision under which the association was obtained."
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Association deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339; dependent permits cannot outlive it."
          },
          "name": {
            "type": "string",
            "description": "Resolved domain name matched by the demand."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network to which this DNS input belongs."
          },
          "source": {
            "$ref": "#/components/schemas/NodeId",
            "description": "Node whose domain demand may use the association."
          }
        },
        "additionalProperties": false
      },
      "TrustedNode": {
        "type": "object",
        "description": "Authenticated key and capability inventory supplied to Server compilation.",
        "required": [
          "controlKey",
          "dataplaneKey",
          "capabilities"
        ],
        "properties": {
          "capabilities": {
            "type": "object",
            "description": "Authenticated acquire declaration; reported State never overrides it.",
            "additionalProperties": {
              "$ref": "#/components/schemas/Capability"
            },
            "propertyNames": {
              "type": "string"
            }
          },
          "controlKey": {
            "$ref": "#/components/schemas/BoundNodeKey",
            "description": "Current bound Ed25519 control public key and generation."
          },
          "dataplaneKey": {
            "$ref": "#/components/schemas/BoundNodeKey",
            "description": "Current bound X25519 data-plane public key and generation."
          }
        },
        "additionalProperties": false
      },
      "UdpTraversalAttemptRequest": {
        "type": "object",
        "description": "Shared accept/cancel CAS body. attemptId is the path parameter.",
        "required": [
          "instance",
          "configuration",
          "stateEpoch",
          "version",
          "digest"
        ],
        "properties": {
          "configuration": {
            "$ref": "#/components/schemas/ConfigurationRef",
            "description": "Participant's exact current execution target, matching its offer socket binding."
          },
          "digest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Immutable offer digest required by the compare-and-swap check."
          },
          "instance": {
            "$ref": "#/components/schemas/InstanceRequest",
            "description": "Current participant runtime instance performing the operation."
          },
          "stateEpoch": {
            "$ref": "#/components/schemas/StateEpoch",
            "description": "State epoch in which the offer was prepared."
          },
          "version": {
            "$ref": "#/components/schemas/Version",
            "description": "Immutable offer version required by the compare-and-swap check."
          }
        },
        "additionalProperties": false
      },
      "UdpTraversalAttempts": {
        "type": "array",
        "items": {
          "type": "object",
          "description": "Current acceptance state paired with the attempt's unchanged Server-issued offer.",
          "required": [
            "offer",
            "phase"
          ],
          "properties": {
            "offer": {
              "$ref": "#/components/schemas/UdpTraversalOffer",
              "description": "Immutable candidate pair, authority binding and original acquisition deadline."
            },
            "phase": {
              "$ref": "#/components/schemas/UdpTraversalPhase",
              "description": "Active only after both actual socket receivers reserved bounded slots."
            }
          },
          "additionalProperties": false
        },
        "maxItems": 32
      },
      "UdpTraversalId": {
        "type": "string",
        "format": "uuid",
        "description": "Short-lived UDP traversal attempt identifier; not a credential.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "UdpTraversalOffer": {
        "type": "object",
        "description": "Server-issued immutable candidate pair for bounded UDP acquisition, valid only until its original deadline.",
        "required": [
          "attemptId",
          "requestId",
          "networkId",
          "stateEpoch",
          "version",
          "digest",
          "nonce",
          "carrier",
          "local",
          "peer",
          "expiresAt"
        ],
        "properties": {
          "attemptId": {
            "$ref": "#/components/schemas/UdpTraversalId",
            "description": "Server-assigned acquisition attempt identifier used by accept and cancel."
          },
          "carrier": {
            "$ref": "#/components/schemas/Carrier",
            "description": "Exact authorized Native UDP adjacency, independent of which participant prepared the offer."
          },
          "digest": {
            "$ref": "#/components/schemas/Digest",
            "description": "Domain-separated digest committing every immutable offer field except this digest itself."
          },
          "expiresAt": {
            "$ref": "#/components/schemas/Timestamp",
            "description": "Fixed minimum of requested/max-window and every authority/fact deadline."
          },
          "local": {
            "$ref": "#/components/schemas/UdpTraversalParticipant",
            "description": "Prepare caller orientation, identical for both recipients."
          },
          "networkId": {
            "$ref": "#/components/schemas/NetworkId",
            "description": "Network containing both authenticated participants."
          },
          "nonce": {
            "$ref": "#/components/schemas/Digest",
            "description": "Random 32 bytes, public attempt separation, never a credential."
          },
          "peer": {
            "$ref": "#/components/schemas/UdpTraversalParticipant",
            "description": "Remote participant relative to the original prepare caller, unchanged for both recipients."
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId",
            "description": "Original preparation request identifier retained for idempotent retries."
          },
          "stateEpoch": {
            "$ref": "#/components/schemas/StateEpoch",
            "description": "State continuity epoch under which the original candidates were selected."
          },
          "version": {
            "$ref": "#/components/schemas/Version",
            "description": "Immutable offer version; accepting does not increment it."
          }
        },
        "additionalProperties": false
      },
      "UdpTraversalParticipant": {
        "type": "object",
        "description": "Server projection of one offer participant's fixed candidate and numeric probe destination.",
        "required": [
          "candidate",
          "address",
          "source"
        ],
        "properties": {
          "address": {
            "type": "string",
            "description": "Original State address, copied without origin/observer/interface disclosure.\nServer and Node must validate a numeric, eligible unicast UDP target."
          },
          "candidate": {
            "$ref": "#/components/schemas/CandidateReference",
            "description": "Exact State observation and socket binding committed by the offer."
          },
          "source": {
            "$ref": "#/components/schemas/EndpointKind",
            "description": "Coarse origin projected from the referenced original State observation."
          }
        },
        "additionalProperties": false
      },
      "UdpTraversalPhase": {
        "type": "string",
        "description": "Server-recorded bilateral acceptance phase, distinct from an authenticated channel being established.",
        "enum": [
          "waiting",
          "active"
        ]
      },
      "UdpTraversalUnavailableReason": {
        "type": "string",
        "description": "Expected authority/CAS races are closed outcomes, not authentication errors.",
        "enum": [
          "scopeChanged",
          "candidateChanged",
          "attemptChanged",
          "attemptExpired",
          "notAvailable",
          "capacity",
          "requestChanged"
        ]
      },
      "UnavailableReason": {
        "type": "string",
        "description": "Condition preventing Server compilation from granting the requested business execution.",
        "enum": [
          "exitUnavailable",
          "unsupportedFamily",
          "egressConflict",
          "unauthorized",
          "noPath",
          "routingCycle",
          "routingLimit",
          "missingCapability",
          "missingDependency",
          "missingDns",
          "invalidProfile",
          "inactiveNode"
        ]
      },
      "UserConfigurationView": {
        "oneOf": [
          {
            "type": "object",
            "description": "Expose the complete redacted network configuration to an authorized manager.",
            "required": [
              "networkId",
              "revision",
              "configuration",
              "type"
            ],
            "properties": {
              "configuration": {
                "$ref": "#/components/schemas/ConfigurationView",
                "description": "Complete network settings without secret material or sensitive header values."
              },
              "networkId": {
                "$ref": "#/components/schemas/NetworkId",
                "description": "Network that owns this configuration snapshot."
              },
              "revision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Exact desired source revision represented by the configuration."
              },
              "type": {
                "type": "string",
                "enum": [
                  "manager"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Expose only the member's permitted node settings.",
            "required": [
              "networkId",
              "revision",
              "nodes",
              "type"
            ],
            "properties": {
              "networkId": {
                "$ref": "#/components/schemas/NetworkId",
                "description": "Network that owns this configuration snapshot."
              },
              "nodes": {
                "type": "object",
                "description": "Permitted node IDs mapped to their redacted desired settings.",
                "additionalProperties": {
                  "$ref": "#/components/schemas/NodeSettingsView"
                },
                "propertyNames": {
                  "type": "string",
                  "format": "uuid",
                  "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
                  "maxLength": 36,
                  "minLength": 36,
                  "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
                }
              },
              "revision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Exact desired source revision represented by the node settings."
              },
              "type": {
                "type": "string",
                "enum": [
                  "member"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Permission-specific desired configuration snapshot for an authenticated network user."
      },
      "UserEntitiesView": {
        "type": "object",
        "description": "User-visible network, node and membership inventory under current permissions.",
        "required": [
          "network",
          "nodes",
          "members"
        ],
        "properties": {
          "members": {
            "type": "object",
            "description": "Visible member summaries keyed by stable network membership ID.",
            "additionalProperties": {
              "$ref": "#/components/schemas/MemberSummary"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Network membership identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          },
          "network": {
            "$ref": "#/components/schemas/NetworkSummary",
            "description": "Network summary and currently available actions."
          },
          "nodes": {
            "type": "object",
            "description": "Visible node inventory keyed by node ID.",
            "additionalProperties": {
              "$ref": "#/components/schemas/NodeInventoryView"
            },
            "propertyNames": {
              "type": "string",
              "format": "uuid",
              "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
              "maxLength": 36,
              "minLength": 36,
              "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            }
          }
        },
        "additionalProperties": false
      },
      "UserEvent": {
        "oneOf": [
          {
            "type": "object",
            "description": "First frame of an initial synchronization batch, containing the visible configuration.",
            "required": [
              "batch",
              "requestId",
              "configuration",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier shared by all snapshots and the ready frame in this batch."
              },
              "configuration": {
                "$ref": "#/components/schemas/UserConfigurationView",
                "description": "Current configuration projected for the authenticated user or Node."
              },
              "progress": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/ConfigurationApplyProgress",
                    "description": "Visible configuration application progress when available; current Server snapshots include it."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "requestId": {
                "$ref": "#/components/schemas/RequestId",
                "description": "Correlation ID of the HTTP request carrying this stream, as in a\nGET response and the `x-request-id` header."
              },
              "type": {
                "type": "string",
                "enum": [
                  "config.snapshot"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Entity inventory frame of the initial synchronization batch.",
            "required": [
              "batch",
              "entities",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier shared with this batch's configuration, state, and ready frames."
              },
              "entities": {
                "$ref": "#/components/schemas/UserEntitiesView",
                "description": "Complete entity inventory within the caller's authorized scope."
              },
              "type": {
                "type": "string",
                "enum": [
                  "entities.snapshot"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "State frame of the initial synchronization batch, retaining original report freshness.",
            "required": [
              "batch",
              "state",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier shared with this batch's configuration, entities, and ready frames."
              },
              "state": {
                "$ref": "#/components/schemas/StateView",
                "description": "Current permission-filtered operational state."
              },
              "type": {
                "type": "string",
                "enum": [
                  "state.snapshot"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Commit marker allowing the client to apply a complete initial snapshot batch.",
            "required": [
              "batch",
              "cursor",
              "heartbeatIntervalMs",
              "disconnectAfterMs",
              "type"
            ],
            "properties": {
              "batch": {
                "type": "string",
                "description": "Identifier of the completed snapshot batch."
              },
              "cursor": {
                "type": "string",
                "description": "Resume cursor to save only after the entire batch has been validated and applied."
              },
              "disconnectAfterMs": {
                "type": "integer",
                "format": "int32",
                "description": "Read-idle timeout in milliseconds after which the client should reconnect.",
                "minimum": 0
              },
              "heartbeatIntervalMs": {
                "type": "integer",
                "format": "int32",
                "description": "Expected interval between idle heartbeat comments, in milliseconds.",
                "minimum": 0
              },
              "type": {
                "type": "string",
                "enum": [
                  "sync.ready"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Incremental operational state projection following the previously applied stream cursor.",
            "required": [
              "stateEpoch",
              "baseCursor",
              "cursor",
              "updated",
              "removed",
              "links",
              "paths",
              "type"
            ],
            "properties": {
              "asOf": {
                "type": [
                  "string",
                  "null"
                ],
                "format": "date-time",
                "description": "Projection time, not a new measurement timestamp. Current Server\nframes include these metadata fields; optionality permits consumers\nto apply previously retained deltas without inventing values."
              },
              "baseCursor": {
                "type": "string",
                "description": "Cursor of the preceding applied stream event, including non-state notices."
              },
              "configurationRevision": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/Revision",
                    "description": "Configuration revision used for this projection; omitted in retained deltas lacking this metadata."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "cursor": {
                "type": "string",
                "description": "Resume cursor to save after this delta has been validated and applied."
              },
              "links": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/AggregatedLink"
                },
                "description": "Replacement collection of currently visible aggregated links."
              },
              "paths": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/AggregatedPath"
                },
                "description": "Replacement collection of currently visible aggregated paths."
              },
              "removed": {
                "type": "array",
                "items": {
                  "$ref": "#/components/schemas/NodeId"
                },
                "description": "Node identifiers to remove from the cached state view."
              },
              "sequence": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/Sequence",
                    "description": "Network committed-change sequence; omitted only in retained deltas lacking this metadata."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "stateEpoch": {
                "$ref": "#/components/schemas/StateEpoch",
                "description": "Epoch identifying the state continuity domain for this update."
              },
              "type": {
                "type": "string",
                "enum": [
                  "state.delta"
                ]
              },
              "updated": {
                "type": "object",
                "description": "Complete replacement state for each updated visible node.",
                "additionalProperties": {
                  "$ref": "#/components/schemas/NodeStateView"
                },
                "propertyNames": {
                  "type": "string",
                  "format": "uuid",
                  "description": "Stable node identifier, independent of its addresses and keys.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
                  "maxLength": 36,
                  "minLength": 36,
                  "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
                }
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Entity replacements and explicit removals following the previously applied stream cursor.",
            "required": [
              "baseCursor",
              "cursor",
              "updated",
              "removed",
              "type"
            ],
            "properties": {
              "baseCursor": {
                "type": "string",
                "description": "Cursor of the preceding applied stream event, including configuration and resource notices."
              },
              "cursor": {
                "type": "string",
                "description": "Resume cursor to save after this delta has been validated and applied."
              },
              "removed": {
                "$ref": "#/components/schemas/EntityRemovals",
                "description": "Explicit entity identifiers to remove from the cached inventory."
              },
              "type": {
                "type": "string",
                "enum": [
                  "entities.delta"
                ]
              },
              "updated": {
                "$ref": "#/components/schemas/UserEntitiesView",
                "description": "Updated entity fragments that replace their corresponding cached entries."
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Notification of a committed configuration revision, independent of Node application progress.",
            "required": [
              "networkId",
              "previousRevision",
              "revision",
              "changedAt",
              "cause",
              "actor",
              "changedPointers",
              "changesComplete",
              "sourceChanged",
              "type"
            ],
            "properties": {
              "actor": {
                "$ref": "#/components/schemas/EventActor",
                "description": "Committing actor, redacted for users without management scope and for Nodes."
              },
              "cause": {
                "$ref": "#/components/schemas/ConfigurationChangeCause",
                "description": "Primary trigger for the configuration change."
              },
              "changedAt": {
                "type": "string",
                "format": "date-time",
                "description": "Time at which the configuration change was committed."
              },
              "changedPointers": {
                "type": "array",
                "items": {
                  "type": "string"
                },
                "description": "Visible changed JSON Pointers, provided as invalidation hints rather than a complete dependency analysis."
              },
              "changesComplete": {
                "type": "boolean",
                "description": "Whether the changed-pointer list is complete rather than redacted or truncated."
              },
              "networkId": {
                "$ref": "#/components/schemas/NetworkId",
                "description": "Network whose saved configuration changed."
              },
              "previousRevision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Desired configuration revision immediately before the commit."
              },
              "requestId": {
                "oneOf": [
                  {
                    "$ref": "#/components/schemas/RequestId",
                    "description": "Committing request's correlation ID, or null when withheld from the subscriber."
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "revision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Newly committed desired configuration revision."
              },
              "sourceChanged": {
                "type": "boolean",
                "description": "Whether the original configuration source text changed."
              },
              "type": {
                "type": "string",
                "enum": [
                  "configuration.changed"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Configuration application progress using the same projection as the progress GET operation.",
            "required": [
              "progress",
              "type"
            ],
            "properties": {
              "progress": {
                "$ref": "#/components/schemas/ConfigurationApplyProgress",
                "description": "Updated application progress for the fixed configuration submission target."
              },
              "type": {
                "type": "string",
                "enum": [
                  "configuration.progress"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "A manager must re-read the normal resource GET view in this network.\nThis notice carries no source URL, material or unscoped artifact data.",
            "required": [
              "networkId",
              "revision",
              "type"
            ],
            "properties": {
              "networkId": {
                "$ref": "#/components/schemas/NetworkId",
                "description": "Network whose authorized resource view should be reread."
              },
              "revision": {
                "$ref": "#/components/schemas/Revision",
                "description": "Configuration revision associated with the resource change notification."
              },
              "type": {
                "type": "string",
                "enum": [
                  "resources.changed"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Subscriber-specific observation task status update.",
            "required": [
              "observation",
              "type"
            ],
            "properties": {
              "observation": {
                "$ref": "#/components/schemas/ObservationTask",
                "description": "Observation task with this subscription's current status and execution context."
              },
              "type": {
                "type": "string",
                "enum": [
                  "observation.status"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Subscriber-specific observation update containing actual measurement results.",
            "required": [
              "observation",
              "type"
            ],
            "properties": {
              "observation": {
                "$ref": "#/components/schemas/ObservationTask",
                "description": "Observation task with authorized samples and their original measurement context."
              },
              "type": {
                "type": "string",
                "enum": [
                  "observation.sample"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Notification that a terminated observation subscription has left the retained view.",
            "required": [
              "subscriptionId",
              "type"
            ],
            "properties": {
              "subscriptionId": {
                "$ref": "#/components/schemas/ObservationSubscriptionId",
                "description": "Subscription whose cached observation view should be removed."
              },
              "type": {
                "type": "string",
                "enum": [
                  "observation.removed"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Notification that incremental continuity was lost and a new full batch is required.",
            "required": [
              "reason",
              "type"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/ResetReason",
                "description": "Reason the previous cursor and cached view can no longer continue incrementally."
              },
              "type": {
                "type": "string",
                "enum": [
                  "sync.reset"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "Final stream notice before closure when an explicit reason can be delivered.",
            "required": [
              "reason",
              "type"
            ],
            "properties": {
              "reason": {
                "$ref": "#/components/schemas/ClosedReason",
                "description": "Authentication, lease, or shutdown condition that ended the stream."
              },
              "type": {
                "type": "string",
                "enum": [
                  "stream.closed"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "One SSE data frame containing a permission-filtered snapshot, update, or stream lifecycle notice."
      },
      "UserId": {
        "type": "string",
        "format": "uuid",
        "description": "Stable user identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "UserView": {
        "type": "object",
        "description": "A user account; this view grants no network or instance privileges.",
        "required": [
          "id",
          "email"
        ],
        "properties": {
          "email": {
            "type": "string",
            "description": "Canonical login identifier, not proof of email ownership."
          },
          "id": {
            "$ref": "#/components/schemas/UserId",
            "description": "Stable account identifier, unchanged by email updates."
          }
        },
        "additionalProperties": false
      },
      "Version": {
        "type": "integer",
        "description": "A positive record version, distinct from a configuration revision.",
        "maximum": 9007199254740991,
        "minimum": 1
      },
      "VisibleReport": {
        "oneOf": [
          {
            "type": "object",
            "description": "The caller may read the complete Node report.",
            "required": [
              "report",
              "view"
            ],
            "properties": {
              "report": {
                "$ref": "#/components/schemas/NodeState",
                "description": "Complete accepted runtime snapshot."
              },
              "view": {
                "type": "string",
                "enum": [
                  "full"
                ]
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
            "description": "The caller may read only execution-related peer facts.",
            "required": [
              "report",
              "view"
            ],
            "properties": {
              "report": {
                "$ref": "#/components/schemas/PeerState",
                "description": "Restricted peer snapshot without local system or interface details."
              },
              "view": {
                "type": "string",
                "enum": [
                  "peer"
                ]
              }
            },
            "additionalProperties": false
          }
        ],
        "description": "Report projection selected by the Server under the caller's current permissions."
      },
      "VlessDialer": {
        "type": "object",
        "description": "Outbound carrier selection and connection options; the Server resolves target listener parameters separately.",
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "connectTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive total connection budget including resolution and required handshakes; defaults to `5s` and is limited by the remaining active lease."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether this dialer may be used for outgoing connections; defaults to true."
          },
          "localAddress": {
            "type": "string",
            "description": "Local outgoing IP address; omitted or empty selects automatically, and a Mesh address cannot bypass underlay protection."
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "description": "Preference from 0–65535, lower first; defaults to 100 for native_udp, 200 for native_tcp, 300 for vless, or 400 for trojan.",
            "minimum": 0
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text rather than a resource key; defaults to an empty string."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          },
          "stream": {
            "$ref": "#/components/schemas/DialerStream",
            "description": "Encapsulation selector, `auto`, `tcp`, or `websocket`; defaults to `auto`, while explicit values filter compatible listeners."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
          },
          "tls": {
            "$ref": "#/components/schemas/DialerTls",
            "description": "TLS client options; omission uses managed trust, TLS 1.2–1.3, empty ALPN, and a `5s` handshake budget."
          },
          "websocket": {
            "$ref": "#/components/schemas/DialerWebSocket",
            "description": "Extra WebSocket client options; omission adds no request headers, tcp selection forbids this field, and null is rejected."
          }
        },
        "additionalProperties": false
      },
      "VlessDialerUpdate": {
        "type": "object",
        "description": "Replacement VLESS dialer options with explicit retention of omitted WebSocket settings.",
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Requested underlay interface; defaults to an empty string for platform selection."
          },
          "connectTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive total resolution, connection and handshake budget; defaults to `5s` and remains bounded by the execution lease."
          },
          "enabled": {
            "type": "boolean",
            "description": "Allow active dialing with this protocol; defaults to true."
          },
          "localAddress": {
            "type": "string",
            "description": "Local outbound IP address; defaults to an empty string for automatic selection and cannot bypass Mesh address protection."
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "description": "Preference in 0–65535, with smaller values preferred; defaults to 300.",
            "minimum": 0
          },
          "remark": {
            "type": "string",
            "description": "Free-form description; defaults to an empty string."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests in bytes; omitted settings use zero for provider defaults."
          },
          "stream": {
            "$ref": "#/components/schemas/DialerStream",
            "description": "Encapsulation filter, defaulting to `auto` to use the selected listener's stream."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "TCP no-delay and keepalive settings; omission uses the shared TCP defaults."
          },
          "tls": {
            "$ref": "#/components/schemas/DialerTls",
            "description": "TLS verification and negotiation settings; defaults to managed trust, TLS 1.2–1.3 and a `5s` handshake budget."
          },
          "websocket": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/DialerWebSocketUpdate",
                "description": "Omission retains previous WebSocket options, null removes them, and a value updates them; incompatible with explicit `tcp`."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "VlessDialerView": {
        "type": "object",
        "description": "Read-only VLESS outbound settings with sensitive WebSocket header values removed.",
        "required": [
          "enabled",
          "remark",
          "priority",
          "connectTimeout",
          "bindInterface",
          "localAddress",
          "socket",
          "tcp",
          "stream",
          "tls"
        ],
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Requested underlay interface; an empty string leaves selection to the platform."
          },
          "connectTimeout": {
            "$ref": "#/components/schemas/PositiveDuration",
            "description": "Positive total connection budget covering resolution, transport and protocol handshakes."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether this dialer may actively acquire a transport."
          },
          "localAddress": {
            "type": "string",
            "description": "Local outbound IP address; an empty string selects it automatically."
          },
          "priority": {
            "type": "integer",
            "format": "int32",
            "description": "Dialing preference in 0–65535, with smaller values preferred.",
            "minimum": 0
          },
          "remark": {
            "type": "string",
            "description": "Free-form description, distinct from the protocol key."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Receive and send buffer requests in bytes, with zero selecting provider defaults."
          },
          "stream": {
            "$ref": "#/components/schemas/DialerStream",
            "description": "`auto` accepts the target listener's stream, while explicit values restrict compatible listeners."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "TCP no-delay and operating-system keepalive settings."
          },
          "tls": {
            "$ref": "#/components/schemas/DialerTls",
            "description": "TLS client verification and negotiation settings for the selected listener's bound trust materials."
          },
          "websocketHeaderNames": {
            "type": [
              "array",
              "null"
            ],
            "items": {
              "type": "string"
            },
            "description": "Names of extra WebSocket request headers; null when no WebSocket options are configured."
          }
        },
        "additionalProperties": false
      },
      "VlessListener": {
        "type": "object",
        "description": "Declared inbound carrier settings for a fixed listening resource, with protocol-specific parameters.",
        "required": [
          "listenPort",
          "tls"
        ],
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Interface in the current controlled underlay; omitted or empty lets the platform choose."
          },
          "enabled": {
            "type": "boolean",
            "description": "Whether to create this listening resource; defaults to true, while false retains configuration without executing it."
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/Endpoint"
            },
            "description": "Explicitly advertised external addresses, which may differ from the bind address; omitted or empty advertises none."
          },
          "flow": {
            "type": "string",
            "description": "VLESS flow selection; only the empty string is accepted and omission defaults to it."
          },
          "listen": {
            "type": "string",
            "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0` and cannot be a DNS name."
          },
          "listenPort": {
            "$ref": "#/components/schemas/Port",
            "description": "Required local listening port in the range 1–65535."
          },
          "remark": {
            "type": "string",
            "description": "Descriptive text; defaults to an empty string and cannot be null."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests; omission uses provider defaults for both receive and send buffers."
          },
          "stream": {
            "$ref": "#/components/schemas/VlessStream",
            "description": "VLESS encapsulation, `tcp` or `websocket`, both over TLS; defaults to `tcp`."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "Outer TCP socket settings; omission uses TCP_NODELAY and the default enabled keepalive settings."
          },
          "tls": {
            "$ref": "#/components/schemas/ListenerTls",
            "description": "Required TLS server settings, including the certificate verification name and material source."
          },
          "uuid": {
            "$ref": "#/components/schemas/SecretString",
            "description": "Hyphenated UUID credential with case-insensitive hexadecimal digits; omission makes the Server generate or retain saved material, while null is rejected."
          },
          "websocket": {
            "$ref": "#/components/schemas/ListenerWebSocket",
            "description": "WebSocket Upgrade settings; omission uses defaults for a websocket stream, while a tcp stream forbids this field and null is rejected."
          }
        },
        "additionalProperties": false
      },
      "VlessListenerUpdate": {
        "type": "object",
        "description": "VLESS listener replacement with retention of omitted materials within the same resource.",
        "required": [
          "listenPort",
          "tls"
        ],
        "properties": {
          "bindInterface": {
            "type": "string",
            "description": "Requested underlay interface; defaults to an empty string for platform selection."
          },
          "enabled": {
            "type": "boolean",
            "description": "Request creation of the fixed listener; defaults to true."
          },
          "endpoints": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/Endpoint"
            },
            "description": "Explicitly advertised external host/port pairs; defaults to an empty list."
          },
          "flow": {
            "type": "string",
            "description": "VLESS flow; only the default empty string is supported."
          },
          "listen": {
            "type": "string",
            "description": "Local IPv4 or IPv6 bind address; defaults to `0.0.0.0`."
          },
          "listenPort": {
            "$ref": "#/components/schemas/Port",
            "description": "Required local listening port in 1–65535."
          },
          "remark": {
            "type": "string",
            "description": "Free-form description; defaults to an empty string."
          },
          "socket": {
            "$ref": "#/components/schemas/SocketOptions",
            "description": "Socket buffer requests in bytes; omitted settings use zero for provider defaults."
          },
          "stream": {
            "$ref": "#/components/schemas/VlessStream",
            "description": "VLESS encapsulation, `tcp` or `websocket`; defaults to `tcp` and both use TLS over TCP."
          },
          "tcp": {
            "$ref": "#/components/schemas/TcpOptions",
            "description": "TCP no-delay and keepalive settings; omission uses the shared TCP defaults."
          },
          "tls": {
            "$ref": "#/components/schemas/ListenerTls",
            "description": "TLS declaration; omitted materials are retained for the same mode, and managed certificates are regenerated when the server name changes."
          },
          "uuid": {
            "$ref": "#/components/schemas/SecretString",
            "description": "Protocol UUID; omission retains this resource's saved UUID or generates one initially, while null is rejected."
          },
          "websocket": {
            "oneOf": [
              {
                "$ref": "#/components/schemas/ListenerWebSocketUpdate",
                "description": "Omission retains this resource's WebSocket options, null removes them, and a value updates them; only allowed with `websocket` stream."
              },
              {
                "type": "null"
              }
            ]
          }
        },
        "additionalProperties": false
      },
      "VlessStream": {
        "type": "string",
        "description": "Concrete VLESS stream encapsulation. Dialer selection such as `auto` is not\nan encapsulation, and neither variant means that its runtime is installed.",
        "enum": [
          "tcp",
          "websocket"
        ]
      },
      "WebSocketHeaders": {
        "type": "object",
        "description": "Additional WebSocket headers, preserving casing. HTTP token names cannot override Host, Connection, Upgrade, Content-Length, Transfer-Encoding or Sec-WebSocket-*, ignoring ASCII case. Handshake and HTTP message framing fields belong to the upgrade implementation. Names must also be unique ignoring ASCII case (semantic validation).",
        "additionalProperties": {
          "type": "string",
          "description": "Header text preserved verbatim, including empty values and horizontal tabs.\nCR/LF, NUL and other Unicode control characters (except HTAB) are rejected.\nUnicode text is represented as UTF-8; protocol-specific semantic constraints\nand request/header size limits remain the consumer's responsibility.",
          "pattern": "^[^\\u0000-\\u0008\\u000A-\\u001F\\u007F-\\u009F]*(?![\\s\\S])"
        },
        "propertyNames": {
          "type": "string",
          "description": "Case-preserving HTTP token for an additional WebSocket header.\nHost, Connection, Upgrade, Content-Length, Transfer-Encoding, and every\nSec-WebSocket-* name are reserved, ignoring ASCII case. Handshake and HTTP\nmessage framing fields belong to the upgrade implementation. No Unicode\nheader names are allowed by HTTP token syntax.",
          "pattern": "^(?!(?:[Hh][Oo][Ss][Tt]|[Cc][Oo][Nn][Nn][Ee][Cc][Tt][Ii][Oo][Nn]|[Uu][Pp][Gg][Rr][Aa][Dd][Ee]|[Cc][Oo][Nn][Tt][Ee][Nn][Tt]-[Ll][Ee][Nn][Gg][Tt][Hh]|[Tt][Rr][Aa][Nn][Ss][Ff][Ee][Rr]-[Ee][Nn][Cc][Oo][Dd][Ii][Nn][Gg])(?![\\s\\S]))(?![Ss][Ee][Cc]-[Ww][Ee][Bb][Ss][Oo][Cc][Kk][Ee][Tt]-)[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])"
        }
      },
      "WebSocketPath": {
        "type": "string",
        "description": "Exact path, preserved without URL decoding or normalization.\nUnicode is permitted, but whitespace, C0/C1 controls, query and fragment\ndelimiters are not. Percent escapes are not decoded by this value type.",
        "pattern": "^/[^\\u0000-\\u0020\\u007F-\\u009F\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000?#]*(?![\\s\\S])"
      },
      "WireSecret": {
        "type": "string",
        "description": "Secret wire string. Serialization and borrowing are explicit; Debug, Display,\nand Clone are deliberately unavailable. The owned buffer is wiped on drop."
      }
    }
  }
} as const;

export const componentSchemas = schemaDocument.components.schemas;

export function schemaFor(name: keyof typeof componentSchemas) {
  return { ...schemaDocument, $ref: `#/components/schemas/${name}` } as const;
}
