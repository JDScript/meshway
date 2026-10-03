// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

export const schemaDocument = {
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "components": {
    "schemas": {
      "AuthOptionsResponse": {
        "type": "object",
        "required": [
          "requestId",
          "registrationEnabled",
          "passwordRules"
        ],
        "properties": {
          "passwordRules": {
            "$ref": "#/components/schemas/PasswordRules"
          },
          "registrationEnabled": {
            "type": "boolean"
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId"
          }
        },
        "additionalProperties": false
      },
      "AuthSessionResponse": {
        "type": "object",
        "required": [
          "requestId",
          "user",
          "session"
        ],
        "properties": {
          "requestId": {
            "$ref": "#/components/schemas/RequestId"
          },
          "session": {
            "$ref": "#/components/schemas/SessionMetadata"
          },
          "user": {
            "$ref": "#/components/schemas/UserView"
          }
        },
        "additionalProperties": false
      },
      "CreateNetworkRequest": {
        "type": "object",
        "required": [
          "requestId",
          "name"
        ],
        "properties": {
          "name": {
            "type": "string"
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId"
          }
        },
        "additionalProperties": false
      },
      "EmptyParams": {
        "type": "object",
        "additionalProperties": false
      },
      "ErrorResponse": {
        "type": "object",
        "required": [
          "requestId",
          "status",
          "problem"
        ],
        "properties": {
          "problem": {
            "$ref": "#/components/schemas/Problem"
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
            "maximum": 65535,
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "LoginRequest": {
        "type": "object",
        "required": [
          "email",
          "password"
        ],
        "properties": {
          "email": {
            "type": "string"
          },
          "password": {
            "$ref": "#/components/schemas/WireSecret"
          }
        },
        "additionalProperties": false
      },
      "MemberSummary": {
        "type": "object",
        "required": [
          "membershipId",
          "userId",
          "email",
          "role",
          "version",
          "joinedAt"
        ],
        "properties": {
          "email": {
            "type": "string"
          },
          "joinedAt": {
            "type": "string",
            "format": "date-time"
          },
          "membershipId": {
            "$ref": "#/components/schemas/MembershipId"
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole"
          },
          "userId": {
            "$ref": "#/components/schemas/UserId"
          },
          "version": {
            "$ref": "#/components/schemas/Version"
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
      "NetworkActions": {
        "type": "object",
        "required": [
          "rename",
          "editConfiguration",
          "manageMembers"
        ],
        "properties": {
          "editConfiguration": {
            "type": "boolean"
          },
          "manageMembers": {
            "type": "boolean"
          },
          "rename": {
            "type": "boolean"
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
        "required": [
          "requestId",
          "items",
          "nextCursor",
          "asOf"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time"
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/NetworkSummary"
            }
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ]
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId"
          }
        },
        "additionalProperties": false
      },
      "NetworkMembersResponse": {
        "type": "object",
        "required": [
          "requestId",
          "items",
          "nextCursor",
          "asOf"
        ],
        "properties": {
          "asOf": {
            "type": "string",
            "format": "date-time"
          },
          "items": {
            "type": "array",
            "items": {
              "$ref": "#/components/schemas/MemberSummary"
            }
          },
          "nextCursor": {
            "type": [
              "string",
              "null"
            ]
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId"
          }
        },
        "additionalProperties": false
      },
      "NetworkResponse": {
        "type": "object",
        "required": [
          "requestId",
          "network"
        ],
        "properties": {
          "network": {
            "$ref": "#/components/schemas/NetworkSummary"
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId"
          }
        },
        "additionalProperties": false
      },
      "NetworkSummary": {
        "type": "object",
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
            "$ref": "#/components/schemas/NetworkActions"
          },
          "createdAt": {
            "type": "string",
            "format": "date-time"
          },
          "currentRevision": {
            "$ref": "#/components/schemas/Revision"
          },
          "id": {
            "$ref": "#/components/schemas/NetworkId"
          },
          "membershipId": {
            "$ref": "#/components/schemas/MembershipId"
          },
          "name": {
            "type": "string"
          },
          "role": {
            "$ref": "#/components/schemas/MembershipRole"
          }
        },
        "additionalProperties": false
      },
      "PasswordLengthUnit": {
        "type": "string",
        "enum": [
          "unicodeCodepoints"
        ]
      },
      "PasswordRules": {
        "type": "object",
        "required": [
          "minCodepoints",
          "maxCodepoints",
          "maxUtf8Bytes",
          "lengthUnit"
        ],
        "properties": {
          "lengthUnit": {
            "$ref": "#/components/schemas/PasswordLengthUnit"
          },
          "maxCodepoints": {
            "type": "integer",
            "format": "int32",
            "maximum": 65535,
            "minimum": 0
          },
          "maxUtf8Bytes": {
            "type": "integer",
            "format": "int32",
            "maximum": 65535,
            "minimum": 0
          },
          "minCodepoints": {
            "type": "integer",
            "format": "int32",
            "maximum": 65535,
            "minimum": 0
          }
        },
        "additionalProperties": false
      },
      "Problem": {
        "oneOf": [
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/PasswordRules"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          },
          {
            "type": "object",
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
                "type": "string"
              },
              "params": {
                "$ref": "#/components/schemas/EmptyParams"
              }
            },
            "additionalProperties": false
          }
        ]
      },
      "RegisterRequest": {
        "type": "object",
        "required": [
          "email",
          "password"
        ],
        "properties": {
          "email": {
            "type": "string"
          },
          "password": {
            "$ref": "#/components/schemas/WireSecret"
          }
        },
        "additionalProperties": false
      },
      "RenameNetworkRequest": {
        "type": "object",
        "required": [
          "requestId",
          "name",
          "expectedName"
        ],
        "properties": {
          "expectedName": {
            "type": "string"
          },
          "name": {
            "type": "string"
          },
          "requestId": {
            "$ref": "#/components/schemas/RequestId"
          }
        },
        "additionalProperties": false
      },
      "RequestId": {
        "type": "string",
        "format": "uuid",
        "description": "Request identifier.\n\n This is an identifier, not a credential. Generation and authorization\n belong to the caller. Parsing rejects aliases rather than normalizing them.",
        "maxLength": 36,
        "minLength": 36,
        "pattern": "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
      },
      "Revision": {
        "type": "integer",
        "description": "A positive configuration revision.",
        "maximum": 9007199254740991,
        "minimum": 1
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
            "$ref": "#/components/schemas/RequestId"
          },
          "session": {
            "$ref": "#/components/schemas/SessionMetadata"
          },
          "token": {
            "$ref": "#/components/schemas/WireSecret"
          },
          "user": {
            "$ref": "#/components/schemas/UserView"
          }
        },
        "additionalProperties": false
      },
      "SessionMetadata": {
        "type": "object",
        "required": [
          "id",
          "createdAt",
          "expiresAt"
        ],
        "properties": {
          "createdAt": {
            "type": "string",
            "format": "date-time"
          },
          "expiresAt": {
            "type": "string",
            "format": "date-time"
          },
          "id": {
            "$ref": "#/components/schemas/SessionId"
          }
        },
        "additionalProperties": false
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
            "$ref": "#/components/schemas/UserId"
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
