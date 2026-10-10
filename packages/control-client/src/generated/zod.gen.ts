// Generated from the Server OpenAPI by scripts/protocol/generate.mjs. Do not edit.

import * as z from "zod";
import { unwrapJsonNumber } from "../json.ts";

function typedMap<Value extends z.ZodType>(
  keySchema: z.ZodType<string>,
  valueSchema: Value,
  bounds: { minProperties?: number; maxProperties?: number },
) {
  return z.custom<Record<string, unknown>>((input: unknown) =>
    input !== null && typeof input === "object" && !Array.isArray(input) &&
    (Object.getPrototypeOf(input) === Object.prototype || Object.getPrototypeOf(input) === null),
    "Expected a JSON object",
  ).transform((input, ctx) => {
    const entries = Object.entries(input);
    let invalid = false;
    for (const [limit, violated] of [
      ["minProperties", bounds.minProperties !== undefined && entries.length < bounds.minProperties],
      ["maxProperties", bounds.maxProperties !== undefined && entries.length > bounds.maxProperties],
    ] as const) {
      if (violated) {
        ctx.addIssue({ code: "custom", message: limit + " exceeded" });
        invalid = true;
      }
    }
    const output: Record<string, z.output<Value>> = {};
    for (const [name, value] of entries) {
      const key = keySchema.safeParse(name);
      const parsed = valueSchema.safeParse(value);
      for (const result of [key, parsed]) {
        if (!result.success) {
          invalid = true;
          for (const issue of result.error.issues) ctx.addIssue({ ...issue, path: [name, ...issue.path] });
        }
      }
      if (key.success && parsed.success) {
        // Defining an own data property preserves __proto__ without invoking
        // Object.prototype's setter or changing the output prototype.
        Object.defineProperty(output, name, { value: parsed.data, enumerable: true, writable: true, configurable: true });
      }
    }
    return invalid ? z.NEVER : output;
  });
}

function rfc3339(value: string): boolean {
  const parts = /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|[+-](\d{2}):(\d{2}))$/.exec(value);
  if (!parts || parts[0].length !== value.length) return false;
  const year = Number(parts[1]), month = Number(parts[2]), day = Number(parts[3]);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
  return days !== undefined && day >= 1 && day <= days && Number(parts[4]) <= 23 &&
    Number(parts[5]) <= 59 && Number(parts[6]) <= 60 &&
    (parts[7] === undefined || (Number(parts[7]) <= 23 && Number(parts[8]) <= 59));
}

function timestampMillis(value: string): boolean {
  if (!rfc3339(value)) return false;
  const parts = /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/.exec(value);
  if (!parts) return false;
  const fraction = parts[7] ?? "";
  if (/[^0]/.test(fraction.slice(3))) return false;
  const date = new Date(0);
  date.setUTCFullYear(Number(parts[1]), Number(parts[2]) - 1, Number(parts[3]));
  const second = Number(parts[6]);
  if (second === 60) return false;
  date.setUTCHours(Number(parts[4]), Number(parts[5]), Math.min(second, 59), Number(fraction.slice(0, 3).padEnd(3, "0")));
  const offset = parts[8] === undefined ? 0 : (Number(parts[9]) * 60 + Number(parts[10])) * (parts[8] === "+" ? 1 : -1);
  const millis = date.getTime() - offset * 60000;
  return millis >= 0 && millis <= 253402300799999;
}

const socketIpv4 = z.ipv4();
const socketIpv6 = z.ipv6();
function socketAddress(value: string): boolean {
  const parts = /^(?:\[([^\]]+)\]|([^:]+)):([0-9]+)$/.exec(value);
  if (!parts || parts[0].length !== value.length) return false;
  const port = Number(parts[3]);
  if (port < 1 || port > 65535) return false;
  const host = parts[1] ?? parts[2];
  if (host === undefined || /[^0-9a-fA-F:.%]/.test(host)) return false;
  if (parts[1] === undefined) return socketIpv4.safeParse(parts[2]).success;
  const separator = parts[1].lastIndexOf("%");
  if (separator < 0) return socketIpv6.safeParse(parts[1]).success;
  const scope = parts[1].slice(separator + 1);
  return scope.length > 0 && !/[^0]/.test(scope) && socketIpv6.safeParse(parts[1].slice(0, separator)).success;
}

export const zUdpTraversalId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zAcceptUdpTraversalPath = z.strictObject({
  ["attemptId"]: zUdpTraversalId
});

export const zNodeId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zTransportProtocol = z.enum(["native_udp","native_tcp","vless","trojan"]);

export const zCarrier = z.strictObject({
  ["dialer"]: zNodeId,
  ["listener"]: zNodeId,
  ["listenerKey"]: z.string(),
  ["protocol"]: zTransportProtocol
});

export const zDigest = z.string().refine(value => [...value].length >= 64, "minLength 64 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[0-9a-f]{64}$", "u"));

export const zTimestamp = z.string().refine(timestampMillis, "Expected nonnegative RFC 3339 exact milliseconds");

export const zVersion = z.int().gte(1).lte(9007199254740991);

export const zInstanceId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zInstanceRequest = z.strictObject({
  ["generation"]: zVersion,
  ["instanceId"]: zInstanceId
});

export const zRevision = z.int().gte(1).lte(9007199254740991);

export const zNetmapRef = z.strictObject({
  ["digest"]: zDigest,
  ["revision"]: zRevision,
  ["serial"]: zVersion
});

export const zResourceRef = z.union([z.strictObject({
  ["key"]: z.string(),
  ["kind"]: z.enum(["listener"])
}), z.strictObject({
  ["key"]: z.string(),
  ["kind"]: z.enum(["dial"])
}), z.strictObject({
  ["key"]: z.string(),
  ["kind"]: z.enum(["coordination"])
})]);

export const zSequence = z.int().gte(0).lte(9007199254740991);

export const zSocketReference = z.strictObject({
  ["instance"]: zInstanceRequest,
  ["netmap"]: zNetmapRef,
  ["nodeId"]: zNodeId,
  ["resource"]: zResourceRef,
  ["socketGeneration"]: zSequence,
  ["socketId"]: z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 256, "maxLength 256 Unicode code points").regex(new RegExp("^[!-~]{1,256}(?![\\s\\S])", "u"))
});

export const zCandidateReference = z.strictObject({
  ["observationDigest"]: zDigest,
  ["socket"]: zSocketReference
});

export const zEndpointKind = z.enum(["configured","interface","stun"]);

export const zUdpTraversalParticipant = z.strictObject({
  ["address"]: z.string(),
  ["candidate"]: zCandidateReference,
  ["source"]: zEndpointKind
});

export const zNetworkId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zRequestId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zStateEpoch = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zUdpTraversalOffer = z.strictObject({
  ["attemptId"]: zUdpTraversalId,
  ["carrier"]: zCarrier,
  ["digest"]: zDigest,
  ["expiresAt"]: zTimestamp,
  ["local"]: zUdpTraversalParticipant,
  ["networkId"]: zNetworkId,
  ["nonce"]: zDigest,
  ["peer"]: zUdpTraversalParticipant,
  ["requestId"]: zRequestId,
  ["stateEpoch"]: zStateEpoch,
  ["version"]: zVersion
});

export const zUdpTraversalUnavailableReason = z.enum(["scopeChanged","candidateChanged","attemptChanged","attemptExpired","notAvailable","capacity","requestChanged"]);

export const zAcceptUdpTraversalResponse = z.union([z.strictObject({
  ["offer"]: zUdpTraversalOffer,
  ["status"]: z.enum(["waiting"])
}), z.strictObject({
  ["offer"]: zUdpTraversalOffer,
  ["status"]: z.enum(["active"])
}), z.strictObject({
  ["reason"]: zUdpTraversalUnavailableReason,
  ["status"]: z.enum(["unavailable"])
})]);

export const zAccessDefault = z.enum(["deny"]);

export const zAccessGrantEvaluation = z.strictObject({
  ["resolutionDependent"]: z.boolean(),
  ["reverse"]: z.boolean(),
  ["rule"]: z.string(),
  ["traffic"]: z.array(z.string())
});

export const zAccessEvaluation = z.strictObject({
  ["allowed"]: z.boolean(),
  ["grants"]: z.array(zAccessGrantEvaluation)
});

export const zAccessRule = z.strictObject({
  ["bidirectional"]: z.boolean().exactOptional(),
  ["dst"]: z.array(z.string()),
  ["ip"]: z.array(z.string()),
  ["remark"]: z.string().exactOptional(),
  ["src"]: z.array(z.string())
});

export const zUserId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zUserView = z.strictObject({
  ["email"]: z.string(),
  ["id"]: zUserId
});

export const zAccountResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["user"]: zUserView
});

export const zSessionId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zAccountSessionView = z.strictObject({
  ["clientLabel"]: z.string().nullable(),
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["current"]: z.boolean(),
  ["delegationClient"]: z.string().nullable(),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["id"]: zSessionId,
  ["lastUsedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable()
});

export const zAccountSessionsResponse = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["items"]: z.array(zAccountSessionView),
  ["nextCursor"]: z.string().nullable(),
  ["requestId"]: zRequestId
});

export const zTransportRole = z.enum(["dial","listen"]);

export const zStreamKind = z.enum(["tcp","websocket","udp"]);

export const zCapability = z.union([z.strictObject({
  ["kind"]: z.enum(["transport"]),
  ["roles"]: z.array(zTransportRole),
  ["streams"]: z.array(zStreamKind)
}), z.strictObject({
  ["kind"]: z.enum(["feature"]),
  ["version"]: z.int().gte(0).lte(2147483647)
})]);

export const zAcquireInstanceRequest = z.strictObject({
  ["capabilities"]: typedMap(z.string(), zCapability, {}),
  ["instanceId"]: zInstanceId
});

export const zActivityAction = z.enum(["account.register","session.login","session.logout","session.revoke","session.revokeOthers","session.revokeAll","session.delegate","account.password","account.passwordReset","account.resetIssued","account.email","network.create","network.rename","configuration.apply","membership.role","membership.join","membership.remove","joinLink.create","joinLink.revoke"]);

export const zActivityActor = z.union([z.strictObject({
  ["email"]: z.string().nullable(),
  ["type"]: z.enum(["user"]),
  ["userId"]: zUserId
}), z.strictObject({
  ["type"]: z.enum(["system"])
})]);

export const zActivityId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zMembershipId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zJoinLinkId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zActivityTarget = z.union([z.strictObject({
  ["previousEmail"]: z.string().nullable(),
  ["type"]: z.enum(["account"]),
  ["userId"]: zUserId
}), z.strictObject({
  ["sessionId"]: zSessionId,
  ["type"]: z.enum(["session"])
}), z.strictObject({
  ["networkId"]: zNetworkId,
  ["type"]: z.enum(["network"])
}), z.strictObject({
  ["membershipId"]: zMembershipId,
  ["type"]: z.enum(["membership"])
}), z.strictObject({
  ["linkId"]: zJoinLinkId,
  ["type"]: z.enum(["joinLink"])
}), z.strictObject({
  ["revision"]: zRevision,
  ["type"]: z.enum(["configuration"])
})]);

export const zActivityEvent = z.strictObject({
  ["action"]: zActivityAction,
  ["actor"]: zActivityActor,
  ["afterRevision"]: zRevision.nullable(),
  ["at"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["beforeRevision"]: zRevision.nullable(),
  ["id"]: zActivityId,
  ["requestId"]: zRequestId,
  ["target"]: zActivityTarget
});

export const zActivityListResponse = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["items"]: z.array(zActivityEvent),
  ["nextCursor"]: z.string().nullable(),
  ["requestId"]: zRequestId
});

export const zActivityResponse = z.strictObject({
  ["activity"]: zActivityEvent,
  ["requestId"]: zRequestId
});

export const zAddressClass = z.enum(["mesh","private","public"]);

export const zAddressPools = z.strictObject({
  ["ipv4"]: z.array(z.string()),
  ["ipv6"]: z.array(z.string())
});

export const zAddressPoolsInput = z.strictObject({
  ["ipv4"]: z.array(z.string()).exactOptional(),
  ["ipv6"]: z.array(z.string()).exactOptional()
});

export const zLinkEvidence = z.enum(["oneSided","matched","conflicting","expired"]);

export const zLinkPhase = z.enum(["preparing","authenticated","closed","failed"]);

export const zReportRef = z.strictObject({
  ["instanceId"]: zInstanceId,
  ["nodeId"]: zNodeId,
  ["reportSequence"]: zSequence
});

export const zAggregatedLink = z.strictObject({
  ["a"]: zNodeId,
  ["b"]: zNodeId,
  ["evidence"]: zLinkEvidence,
  ["inUse"]: z.boolean().nullable().exactOptional(),
  ["initiatorNodeId"]: zNodeId,
  ["linkId"]: z.string(),
  ["phase"]: zLinkPhase,
  ["reports"]: z.array(zReportRef),
  ["since"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["transport"]: z.string()
});

export const zPathEvidence = z.enum(["selected","sent","peerConfirmed","delivered"]);

export const zReporterRole = z.enum(["origin","relay","target","exit"]);

export const zPathState = z.strictObject({
  ["evidence"]: zPathEvidence,
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["netmap"]: zNetmapRef,
  ["nodes"]: z.array(zNodeId),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["reporterRole"]: zReporterRole
});

export const zAggregatedPath = z.strictObject({
  ["pathId"]: z.string(),
  ["reporter"]: zReportRef,
  ["state"]: zPathState
});

export const zApplicationPhase = z.enum(["pending","saved","applied","failed"]);

export const zApplyConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zApplyFailure = z.enum(["validation","storage","listeners","underlay","tun","dns","dataplane"]);

export const zApplyPhase = z.enum(["receiving","validating","saved","applying","applied","failed"]);

export const zEnrollmentRequestId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zApproveDeviceAuthorizationPath = z.strictObject({
  ["enrollmentRequestId"]: zEnrollmentRequestId
});

export const zApproveEnrollmentRequest = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zArtifactId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zArtifactSummary = z.strictObject({
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["digest"]: zDigest,
  ["entries"]: z.int().gte(0).lte(2147483647),
  ["id"]: zArtifactId
});

export const zNetworkNameTrim = z.enum(["unicodeWhiteSpace"]);

export const zNetworkNameRules = z.strictObject({
  ["forbidControlCharacters"]: z.boolean(),
  ["maxCodepoints"]: z.int().gte(0).lte(65535),
  ["maxInputUtf8Bytes"]: z.int().gte(0).lte(65535),
  ["minCodepoints"]: z.int().gte(0).lte(65535),
  ["trim"]: zNetworkNameTrim
});

export const zPasswordLengthUnit = z.enum(["unicodeCodepoints"]);

export const zPasswordRules = z.strictObject({
  ["lengthUnit"]: zPasswordLengthUnit,
  ["maxCodepoints"]: z.int().gte(0).lte(65535),
  ["maxUtf8Bytes"]: z.int().gte(0).lte(65535),
  ["minCodepoints"]: z.int().gte(0).lte(65535)
});

export const zAuthOptionsResponse = z.strictObject({
  ["networkNameRules"]: zNetworkNameRules,
  ["passwordRules"]: zPasswordRules,
  ["registrationEnabled"]: z.boolean(),
  ["requestId"]: zRequestId
});

export const zSessionMetadata = z.strictObject({
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["id"]: zSessionId
});

export const zAuthSessionResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["session"]: zSessionMetadata,
  ["user"]: zUserView
});

export const zCountryProfile = z.strictObject({
  ["countries"]: z.array(z.string()),
  ["profile"]: z.string()
});

export const zAutoProfile = z.strictObject({
  ["fallback"]: z.string(),
  ["rules"]: z.array(zCountryProfile)
});

export const zKeyId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zKeyAlgorithm = z.enum(["ed25519","x25519"]);

export const zNodePublicKey = z.strictObject({
  ["algorithm"]: zKeyAlgorithm,
  ["bytes"]: zDigest
});

export const zBoundNodeKey = z.strictObject({
  ["generation"]: zVersion,
  ["id"]: zKeyId,
  ["publicKey"]: zNodePublicKey
});

export const zCancelEnrollmentRequestPath = z.strictObject({
  ["enrollmentRequestId"]: zEnrollmentRequestId
});

export const zObservationSubscriptionId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zCancelNodeObservationPath = z.strictObject({
  ["subscriptionId"]: zObservationSubscriptionId
});

export const zCancelObservationPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["subscriptionId"]: zObservationSubscriptionId
});

export const zCancelUdpTraversalPath = z.strictObject({
  ["attemptId"]: zUdpTraversalId
});

export const zCancelUdpTraversalResponse = z.union([z.strictObject({
  ["status"]: z.enum(["cancelled"])
}), z.strictObject({
  ["reason"]: zUdpTraversalUnavailableReason,
  ["status"]: z.enum(["unavailable"])
})]);

export const zCandidateSource = z.enum(["configured","interface","stun"]);

export const zChallengeId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zWireSecret = z.string();

export const zChangeEmailRequest = z.strictObject({
  ["email"]: z.string(),
  ["password"]: zWireSecret
});

export const zChangeKind = z.enum(["added","removed","changed"]);

export const zChangeMemberRolePath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["membershipId"]: zMembershipId
});

export const zMembershipRole = z.enum(["owner","admin","member"]);

export const zChangeMemberRoleRequest = z.strictObject({
  ["expectedVersion"]: zVersion,
  ["role"]: zMembershipRole
});

export const zChangePasswordRequest = z.strictObject({
  ["currentPassword"]: zWireSecret,
  ["password"]: zWireSecret
});

export const zSecretString = z.string();

export const zPositiveDuration = z.string().regex(new RegExp("^(?:(?:[1-9][0-9]{0,18}|1[0-7][0-9]{18}|18[0-3][0-9]{17}|184[0-3][0-9]{16}|1844[0-5][0-9]{15}|18446[0-6][0-9]{14}|184467[0-3][0-9]{13}|1844674[0-3][0-9]{12}|184467440[0-6][0-9]{10}|1844674407[0-2][0-9]{9}|18446744073[0-6][0-9]{8}|1844674407370[0-8][0-9]{6}|18446744073709[0-4][0-9]{5}|184467440737095[0-4][0-9]{4}|1844674407370955[0-0][0-9]{3}|18446744073709551[0-5][0-9]{2}|184467440737095516[0-0][0-9]{1}|1844674407370955161[0-4][0-9]{0}|18446744073709551615)ms|(?:[1-9][0-9]{0,15}|1[0-7][0-9]{15}|18[0-3][0-9]{14}|184[0-3][0-9]{13}|1844[0-5][0-9]{12}|18446[0-6][0-9]{11}|184467[0-3][0-9]{10}|1844674[0-3][0-9]{9}|184467440[0-6][0-9]{7}|1844674407[0-2][0-9]{6}|18446744073[0-6][0-9]{5}|1844674407370[0-8][0-9]{3}|18446744073709[0-4][0-9]{2}|184467440737095[0-4][0-9]{1}|1844674407370955[0-0][0-9]{0}|18446744073709551)s|(?:[1-9][0-9]{0,13}|[1-2][0-9]{14}|30[0-6][0-9]{12}|307[0-3][0-9]{11}|3074[0-3][0-9]{10}|30744[0-4][0-9]{9}|307445[0-6][0-9]{8}|3074457[0-2][0-9]{7}|30744573[0-3][0-9]{6}|307445734[0-4][0-9]{5}|3074457345[0-5][0-9]{4}|30744573456[0-0][0-9]{3}|307445734561[0-7][0-9]{2}|3074457345618[0-1][0-9]{1}|30744573456182[0-4][0-9]{0}|307445734561825)m|(?:[1-9][0-9]{0,11}|[1-4][0-9]{12}|5[0-0][0-9]{11}|51[0-1][0-9]{10}|512[0-3][0-9]{9}|51240[0-8][0-9]{7}|512409[0-4][0-9]{6}|5124095[0-4][0-9]{5}|51240955[0-6][0-9]{4}|512409557[0-5][0-9]{3}|51240955760[0-2][0-9]{1}|5124095576030)h)(?![\\s\\S])", "u"));

export const zTlsVersion = z.enum(["1.2","1.3"]);

export const zTlsTrust = z.enum(["managed"]);

export const zDialerTls = z.strictObject({
  ["alpn"]: z.array(z.string()).exactOptional(),
  ["handshakeTimeout"]: zPositiveDuration.exactOptional(),
  ["maxVersion"]: zTlsVersion.exactOptional(),
  ["minVersion"]: zTlsVersion.exactOptional(),
  ["trust"]: zTlsTrust.exactOptional()
});

export const zClientTls = z.strictObject({
  ["certificateChain"]: zSecretString,
  ["options"]: zDialerTls,
  ["serverName"]: z.string()
});

export const zCloseReason = z.enum(["idle","remote_closed","timeout","withdrawn","local_failure"]);

export const zClosedReason = z.enum(["session_expired","membership_revoked","node_lease_expired","server_shutdown"]);

export const zCodeChallengeMethod = z.enum(["S256"]);

export const zCompareConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zCompareConfigurationRequest = z.strictObject({
  ["fromRevision"]: zRevision,
  ["toRevision"]: zRevision
});

export const zPort = z.int().gte(1).lte(65535);

export const zDialEndpointSource = z.enum(["configured","local","stun"]);

export const zDialEndpoint = z.strictObject({
  ["host"]: z.string(),
  ["port"]: zPort,
  ["source"]: zDialEndpointSource
});

export const zSocketOptions = z.strictObject({
  ["receiveBuffer"]: z.int().gte(0).lte(2147483647).exactOptional(),
  ["sendBuffer"]: z.int().gte(0).lte(2147483647).exactOptional()
});

export const zNativeUdpDialer = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional()
});

export const zTcpKeepAlive = z.strictObject({
  ["count"]: z.int().gte(0).lte(2147483647).exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["idle"]: zPositiveDuration.exactOptional(),
  ["interval"]: zPositiveDuration.exactOptional()
});

export const zTcpOptions = z.strictObject({
  ["keepAlive"]: zTcpKeepAlive.exactOptional(),
  ["noDelay"]: z.boolean().exactOptional()
});

export const zNativeTcpDialer = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional()
});

export const zDialerStream = z.enum(["auto","tcp","websocket"]);

export const zWebSocketHeaders = typedMap(z.string().regex(new RegExp("^(?!(?:[Hh][Oo][Ss][Tt]|[Cc][Oo][Nn][Nn][Ee][Cc][Tt][Ii][Oo][Nn]|[Uu][Pp][Gg][Rr][Aa][Dd][Ee]|[Cc][Oo][Nn][Tt][Ee][Nn][Tt]-[Ll][Ee][Nn][Gg][Tt][Hh]|[Tt][Rr][Aa][Nn][Ss][Ff][Ee][Rr]-[Ee][Nn][Cc][Oo][Dd][Ii][Nn][Gg])(?![\\s\\S]))(?![Ss][Ee][Cc]-[Ww][Ee][Bb][Ss][Oo][Cc][Kk][Ee][Tt]-)[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])", "u")), z.string().regex(new RegExp("^[^\\u0000-\\u0008\\u000A-\\u001F\\u007F-\\u009F]*(?![\\s\\S])", "u")), {});

export const zDialerWebSocket = z.strictObject({
  ["headers"]: zWebSocketHeaders.exactOptional()
});

export const zVlessDialer = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zDialerStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zDialerTls.exactOptional(),
  ["websocket"]: zDialerWebSocket.exactOptional()
});

export const zVlessStream = z.enum(["tcp","websocket"]);

export const zWebSocketPath = z.string().regex(new RegExp("^/[^\\u0000-\\u0020\\u007F-\\u009F\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000?#]*(?![\\s\\S])", "u"));

export const zListenerWebSocket = z.strictObject({
  ["host"]: z.string().exactOptional(),
  ["path"]: zWebSocketPath.exactOptional(),
  ["responseHeaders"]: zWebSocketHeaders.exactOptional()
});

export const zTrojanDialer = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zDialerTls.exactOptional()
});

export const zDialParameters = z.union([z.strictObject({
  ["options"]: zNativeUdpDialer,
  ["type"]: z.enum(["nativeUdp"])
}), z.strictObject({
  ["options"]: zNativeTcpDialer,
  ["type"]: z.enum(["nativeTcp"])
}), z.strictObject({
  ["flow"]: z.string(),
  ["options"]: zVlessDialer,
  ["stream"]: zVlessStream,
  ["tls"]: zClientTls,
  ["type"]: z.enum(["vless"]),
  ["uuid"]: zSecretString,
  ["websocket"]: zListenerWebSocket.nullable().exactOptional()
}), z.strictObject({
  ["options"]: zTrojanDialer,
  ["password"]: zSecretString,
  ["tls"]: zClientTls,
  ["type"]: z.enum(["trojan"])
})]);

export const zDialCandidate = z.strictObject({
  ["endpoints"]: z.array(zDialEndpoint),
  ["listenerKey"]: z.string(),
  ["parameters"]: zDialParameters,
  ["peer"]: zNodeId,
  ["priority"]: z.int().gte(0).lte(2147483647)
});

export const zExecutionAddress = z.string();

export const zLogicalPath = z.strictObject({
  ["dialCandidates"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), z.array(z.string()), {}),
  ["nodes"]: z.array(zNodeId)
});

export const zExitFamily = z.enum(["ipv4","ipv6"]);

export const zExitNat = z.enum(["masquerade"]);

export const zExitRole = z.strictObject({
  ["family"]: zExitFamily,
  ["generation"]: zDigest,
  ["nat"]: zExitNat
});

export const zNodeAddresses = z.strictObject({
  ["ipv4"]: z.ipv4().nullable(),
  ["ipv6"]: z.ipv6().nullable()
});

export const zDnsTransport = z.enum(["udp","tcp"]);

export const zDnsService = z.strictObject({
  ["address"]: zExecutionAddress,
  ["exit"]: zNodeId,
  ["generation"]: zDigest,
  ["maxPending"]: z.int().gte(0).lte(2147483647),
  ["path"]: zLogicalPath,
  ["profile"]: z.string(),
  ["role"]: zExitRole,
  ["source"]: zNodeId,
  ["sourceAddresses"]: zNodeAddresses,
  ["timeoutMs"]: z.int().gte(0).lte(2147483647),
  ["transports"]: z.array(zDnsTransport)
});

export const zDnsEgressUnavailable = z.enum(["exitPlanRequired","exitUnavailable","noPath","unauthorized"]);

export const zDnsPlan = z.union([z.strictObject({
  ["type"]: z.enum(["noExit"])
}), z.strictObject({
  ["service"]: zDnsService,
  ["type"]: z.enum(["viaExit"])
}), z.strictObject({
  ["services"]: z.array(zDnsService),
  ["type"]: z.enum(["serveExitResolver"])
}), z.strictObject({
  ["reason"]: zDnsEgressUnavailable,
  ["type"]: z.enum(["unavailable"])
})]);

export const zResolverAction = z.enum(["direct","unmanaged","exit","rejected"]);

export const zNetmapDns = z.strictObject({
  ["egress"]: zDnsPlan,
  ["meshHosts"]: typedMap(z.string(), zNodeAddresses, {}),
  ["profile"]: z.string(),
  ["resolvers"]: typedMap(z.string(), zResolverAction, {})
});

export const zUnavailableReason = z.enum(["exitUnavailable","unsupportedFamily","egressConflict","unauthorized","noPath","routingCycle","routingLimit","missingCapability","missingDependency","missingDns","invalidProfile","inactiveNode"]);

export const zEgressCapturePlan = z.strictObject({
  ["defaultCapture"]: z.boolean(),
  ["directIpv4"]: z.array(z.string()),
  ["generation"]: zDigest,
  ["managedIpv4"]: z.array(z.string()),
  ["sinkIpv6"]: z.array(z.string()),
  ["unavailable"]: zUnavailableReason.nullable().exactOptional(),
  ["unmanagedIpv4"]: z.array(z.string())
});

export const zEgressAction = z.union([z.strictObject({
  ["nodes"]: z.array(zNodeId),
  ["type"]: z.enum(["exit"]),
  ["via"]: z.array(zNodeId)
}), z.strictObject({
  ["type"]: z.enum(["direct"])
}), z.strictObject({
  ["type"]: z.enum(["unmanaged"])
})]);

export const zEgressRoute = z.strictObject({
  ["action"]: zEgressAction,
  ["prefix"]: z.string()
});

export const zEgressPlan = z.strictObject({
  ["capture"]: zEgressCapturePlan,
  ["routes"]: z.array(zEgressRoute)
});

export const zVlessDialerView = z.strictObject({
  ["bindInterface"]: z.string(),
  ["connectTimeout"]: zPositiveDuration,
  ["enabled"]: z.boolean(),
  ["localAddress"]: z.string(),
  ["remark"]: z.string(),
  ["socket"]: zSocketOptions,
  ["stream"]: zDialerStream,
  ["tcp"]: zTcpOptions,
  ["tls"]: zDialerTls,
  ["websocketHeaderNames"]: z.array(z.string()).nullable().exactOptional()
});

export const zDialerSetView = z.strictObject({
  ["native_tcp"]: zNativeTcpDialer.nullable().exactOptional(),
  ["native_udp"]: zNativeUdpDialer.nullable().exactOptional(),
  ["trojan"]: zTrojanDialer.nullable().exactOptional(),
  ["vless"]: zVlessDialerView.nullable().exactOptional()
});

export const zTopologyDefault = z.enum(["allow","deny"]);

export const zDefaultsView = z.strictObject({
  ["access"]: zAccessDefault,
  ["dialers"]: zDialerSetView,
  ["topology"]: zTopologyDefault
});

export const zDnsServer = z.strictObject({
  ["server"]: z.string(),
  ["serverPort"]: zPort.exactOptional(),
  ["type"]: zDnsTransport,
  ["useWithExit"]: z.boolean().exactOptional()
});

export const zDnsConfiguration = z.strictObject({
  ["default"]: z.array(z.string()).exactOptional(),
  ["domain"]: z.string(),
  ["magicDNS"]: z.boolean().exactOptional(),
  ["overrideLocal"]: z.boolean().exactOptional(),
  ["routes"]: typedMap(z.string(), z.array(z.string()), {}).exactOptional(),
  ["searchDomains"]: z.array(z.string()).exactOptional(),
  ["servers"]: typedMap(z.string(), zDnsServer, {}).exactOptional()
});

export const zNodeIdentity = z.union([z.strictObject({
  ["type"]: z.enum(["user"]),
  ["user"]: z.string()
}), z.strictObject({
  ["tags"]: z.array(z.string()),
  ["type"]: z.enum(["tag"])
})]);

export const zEndpoint = z.strictObject({
  ["host"]: z.string(),
  ["port"]: zPort
});

export const zTlsMode = z.enum(["managed","provided"]);

export const zListenerTlsView = z.strictObject({
  ["alpn"]: z.array(z.string()),
  ["certificatePresent"]: z.boolean(),
  ["handshakeTimeout"]: zPositiveDuration,
  ["maxVersion"]: zTlsVersion,
  ["minVersion"]: zTlsVersion,
  ["mode"]: zTlsMode,
  ["privateKeyPresent"]: z.boolean(),
  ["serverName"]: z.string()
});

export const zListenerWebSocketView = z.strictObject({
  ["host"]: z.string().nullable().exactOptional(),
  ["path"]: zWebSocketPath,
  ["responseHeaderNames"]: z.array(z.string())
});

export const zListenerView = z.strictObject({
  ["bindInterface"]: z.string(),
  ["credentialPresent"]: z.boolean(),
  ["enabled"]: z.boolean(),
  ["endpoints"]: z.array(zEndpoint),
  ["flow"]: z.string().nullable().exactOptional(),
  ["listen"]: z.string(),
  ["listenPort"]: z.int().gte(0).lte(2147483647),
  ["protocol"]: zTransportProtocol,
  ["remark"]: z.string(),
  ["socket"]: zSocketOptions,
  ["stream"]: zVlessStream.nullable().exactOptional(),
  ["tcp"]: zTcpOptions.nullable().exactOptional(),
  ["tls"]: zListenerTlsView.nullable().exactOptional(),
  ["websocket"]: zListenerWebSocketView.nullable().exactOptional()
});

export const zNodeSettingsView = z.strictObject({
  ["addresses"]: zNodeAddresses,
  ["dialers"]: zDialerSetView.exactOptional(),
  ["enabled"]: z.boolean(),
  ["exitEnabled"]: z.boolean(),
  ["hostname"]: z.string(),
  ["identity"]: zNodeIdentity,
  ["listeners"]: typedMap(z.string(), zListenerView, {}),
  ["remark"]: z.string()
});

export const zProfileAssignment = z.strictObject({
  ["allowed"]: z.array(z.string()),
  ["default"]: z.string(),
  ["locked"]: z.boolean().exactOptional(),
  ["target"]: z.array(z.string())
});

export const zRouteAction = z.union([z.strictObject({
  ["nodes"]: z.array(z.string()),
  ["type"]: z.enum(["exit"]),
  ["via"]: z.array(z.string()).exactOptional()
}), z.strictObject({
  ["type"]: z.enum(["direct"])
}), z.strictObject({
  ["type"]: z.enum(["unmanaged"])
})]);

export const zRuleAction = z.union([z.strictObject({
  ["nodes"]: z.array(z.string()),
  ["type"]: z.enum(["exit"]),
  ["via"]: z.array(z.string()).exactOptional()
}), z.strictObject({
  ["type"]: z.enum(["direct"])
})]);

export const zRouteMatch = z.strictObject({
  ["domain"]: z.array(z.string()).exactOptional(),
  ["domainSuffix"]: z.array(z.string()).exactOptional(),
  ["ipCIDR"]: z.array(z.string()).exactOptional(),
  ["ruleSets"]: z.array(z.string()).exactOptional()
});

export const zEgressRule = z.strictObject({
  ["action"]: zRuleAction,
  ["match"]: zRouteMatch,
  ["remark"]: z.string().exactOptional()
});

export const zRoutingProfile = z.strictObject({
  ["fallback"]: zRouteAction,
  ["rules"]: z.array(zEgressRule).exactOptional()
});

export const zEgressRouting = z.strictObject({
  ["assignments"]: z.array(zProfileAssignment).exactOptional(),
  ["auto"]: zAutoProfile.exactOptional(),
  ["profiles"]: typedMap(z.string(), zRoutingProfile, {}).exactOptional(),
  ["rules"]: z.array(zEgressRule).exactOptional()
});

export const zMeshRoute = z.strictObject({
  ["dst"]: z.array(z.string()),
  ["remark"]: z.string().exactOptional(),
  ["src"]: z.array(z.string()),
  ["via"]: z.array(z.string())
});

export const zRoutingConfiguration = z.strictObject({
  ["egress"]: zEgressRouting.exactOptional(),
  ["mesh"]: z.array(zMeshRoute).exactOptional()
});

export const zRuleSetFormat = z.enum(["sing-box-source","mihomo-domain","mihomo-ipcidr"]);

export const zRuleSet = z.union([z.strictObject({
  ["domain"]: z.array(z.string()).exactOptional(),
  ["domainSuffix"]: z.array(z.string()).exactOptional(),
  ["ipCIDR"]: z.array(z.string()).exactOptional(),
  ["type"]: z.enum(["inline"])
}), z.strictObject({
  ["format"]: zRuleSetFormat,
  ["type"]: z.enum(["remote"]),
  ["updateInterval"]: zPositiveDuration.exactOptional(),
  ["url"]: z.string()
})]);

export const zTopologyAction = z.enum(["allow","block"]);

export const zTopologyRule = z.strictObject({
  ["a"]: z.string(),
  ["action"]: zTopologyAction,
  ["b"]: z.string(),
  ["keepalive"]: z.boolean().exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["transports"]: z.array(zTransportProtocol).exactOptional()
});

export const zConfigurationView = z.strictObject({
  ["access"]: z.array(zAccessRule),
  ["addressPools"]: zAddressPools,
  ["defaults"]: zDefaultsView,
  ["dns"]: zDnsConfiguration,
  ["groups"]: typedMap(z.string(), z.array(z.string()), {}),
  ["nodes"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zNodeSettingsView, {}),
  ["routing"]: zRoutingConfiguration,
  ["ruleSets"]: typedMap(z.string(), zRuleSet, {}),
  ["tagsOwners"]: typedMap(z.string(), z.array(z.string()), {}),
  ["topology"]: z.array(zTopologyRule)
});

export const zTrustedNode = z.strictObject({
  ["capabilities"]: typedMap(z.string(), zCapability, {}),
  ["controlKey"]: zBoundNodeKey,
  ["dataplaneKey"]: zBoundNodeKey
});

export const zRuleSetContent = z.strictObject({
  ["artifact"]: zArtifactId,
  ["domain"]: z.array(z.string()),
  ["domainSuffix"]: z.array(z.string()),
  ["ipCIDR"]: z.array(z.string()),
  ["networkId"]: zNetworkId
});

export const zUserBinding = z.strictObject({
  ["membershipId"]: zMembershipId,
  ["userId"]: zUserId
});

export const zSubjectContext = z.strictObject({
  ["members"]: typedMap(z.string(), zUserBinding, {}),
  ["nodeUsers"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zUserBinding, {}),
  ["users"]: typedMap(z.string(), zUserBinding, {})
});

export const zProjectedConfiguration = z.strictObject({
  ["configuration"]: zConfigurationView,
  ["inventory"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zTrustedNode, {}),
  ["ruleSets"]: typedMap(z.string(), zRuleSetContent, {}),
  ["selectedProfiles"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), z.string(), {}),
  ["selectionVersions"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zVersion, {}),
  ["subjects"]: zSubjectContext
});

export const zTrojanStream = z.enum(["tcp"]);

export const zPreparedListener = z.union([z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["type"]: z.enum(["native_udp"])
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["type"]: z.enum(["native_tcp"])
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["flow"]: z.string().exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zVlessStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: z.strictObject({
  ["alpn"]: z.array(z.string()).exactOptional(),
  ["certificate"]: zSecretString,
  ["handshakeTimeout"]: zPositiveDuration.exactOptional(),
  ["maxVersion"]: zTlsVersion.exactOptional(),
  ["minVersion"]: zTlsVersion.exactOptional(),
  ["mode"]: zTlsMode.exactOptional(),
  ["privateKey"]: zSecretString,
  ["serverName"]: z.string()
}),
  ["type"]: z.enum(["vless"]),
  ["uuid"]: zSecretString,
  ["websocket"]: zListenerWebSocket.exactOptional()
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["password"]: zSecretString,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zTrojanStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: z.strictObject({
  ["alpn"]: z.array(z.string()).exactOptional(),
  ["certificate"]: zSecretString,
  ["handshakeTimeout"]: zPositiveDuration.exactOptional(),
  ["maxVersion"]: zTlsVersion.exactOptional(),
  ["minVersion"]: zTlsVersion.exactOptional(),
  ["mode"]: zTlsMode.exactOptional(),
  ["privateKey"]: zSecretString,
  ["serverName"]: z.string()
}),
  ["type"]: z.enum(["trojan"])
})]);

export const zNetmapSelf = z.strictObject({
  ["addresses"]: zNodeAddresses,
  ["enabled"]: z.boolean(),
  ["exitEnabled"]: z.boolean(),
  ["hostname"]: z.string(),
  ["identity"]: zNodeIdentity,
  ["listeners"]: typedMap(z.string(), zPreparedListener, {}),
  ["meshAddressPools"]: zAddressPools,
  ["selfKeys"]: zTrustedNode
});

export const zNodeNetmap = z.strictObject({
  ["compilerVersion"]: z.string(),
  ["dependencyDigest"]: zDigest,
  ["dialers"]: typedMap(z.string(), zDialCandidate, {}),
  ["dns"]: zNetmapDns,
  ["egress"]: zEgressPlan.nullable(),
  ["exitRole"]: zExitRole.nullable(),
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId,
  ["projection"]: zProjectedConfiguration,
  ["revision"]: zRevision,
  ["self"]: zNetmapSelf,
  ["serial"]: zVersion
});

export const zCompiledNetmap = z.strictObject({
  ["digest"]: zDigest,
  ["netmap"]: zNodeNetmap
});

export const zConfigurationAppliedResponse = z.strictObject({
  ["deletedNodeIds"]: z.array(zNodeId),
  ["networkId"]: zNetworkId,
  ["requestId"]: zRequestId,
  ["revision"]: zRevision,
  ["targetCount"]: z.int().gte(0).lte(2147483647)
});

export const zReceiptFreshness = z.enum(["current","expired","awaitingInstance","superseded"]);

export const zConfigurationNodeProgress = z.strictObject({
  ["attempt"]: zVersion.nullable().exactOptional(),
  ["freshness"]: zReceiptFreshness,
  ["instanceId"]: zInstanceId.nullable().exactOptional(),
  ["phase"]: zApplicationPhase,
  ["receivedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["sequence"]: zSequence.nullable().exactOptional(),
  ["target"]: zNetmapRef
});

export const zProgressScope = z.enum(["full","partial"]);

export const zConfigurationProgressSummary = z.strictObject({
  ["applied"]: z.int().gte(0).lte(2147483647),
  ["failed"]: z.int().gte(0).lte(2147483647),
  ["pending"]: z.int().gte(0).lte(2147483647),
  ["saved"]: z.int().gte(0).lte(2147483647),
  ["targetCount"]: z.int().gte(0).lte(2147483647)
});

export const zConfigurationApplyProgress = z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodes"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zConfigurationNodeProgress, {}),
  ["revision"]: zRevision,
  ["scope"]: zProgressScope,
  ["summary"]: zConfigurationProgressSummary
});

export const zConfigurationChange = z.strictObject({
  ["kind"]: zChangeKind,
  ["path"]: z.string()
});

export const zConfigurationChangeCause = z.enum(["networkCreated","edit","restore","enrollment","nodeDeleted","emailSync","memberRemoved","identityTransition"]);

export const zConfigurationComparison = z.strictObject({
  ["bindingsChanged"]: z.boolean(),
  ["changes"]: z.array(zConfigurationChange),
  ["configurationChanged"]: z.boolean(),
  ["fromRevision"]: zRevision,
  ["requestId"]: zRequestId,
  ["sourceChanged"]: z.boolean(),
  ["toRevision"]: zRevision
});

export const zIssueCode = z.enum(["invalid_jsonc","invalid_shape","incomplete_version","unknown_user","invalid_group_name","invalid_tag_name","unknown_group","unknown_tag","duplicate_tag","identity_membership_missing","empty_tag_identity","unknown_node","unknown_node_selector","retained_membership_inactive","empty_selector_set","bidirectional_requires_mesh","invalid_port_set","invalid_domain_suffix","invalid_domain","noncanonical_cidr","invalid_dns_domain","invalid_node_dns_name","dns_override_requires_default","unknown_resolver","invalid_resolver","empty_resolver_set","duplicate_dns_suffix","invalid_dns_suffix","invalid_hostname","duplicate_hostname","empty_listener_key","invalid_transport","invalid_listener_materials","overlapping_pools","invalid_pool","address_outside_pool","duplicate_address","unknown_reassignment_target","no_address_available","repeated_required_node","empty_exit_set","repeated_exit","exit_not_enabled","invalid_destination_cidr","empty_route_match","unknown_rule_set","invalid_rule_source","empty_rule_set_key","empty_inline_rule_set","rule_set_budget","rule_set_unavailable","rule_set_timeout","reserved_profile_name","unknown_profile","default_not_allowed","auto_target_not_allowed","auto_target_unknown","conflicting_assignments","invalid_country_code","exit_outside_access","global_exit_requires_internet","exit_fallback_requires_internet","netmap_limit","shadowed_rule"]);

export const zConfigurationIssue = z.strictObject({
  ["code"]: zIssueCode,
  ["message"]: z.string(),
  ["params"]: typedMap(z.string(), z.string(), {}),
  ["path"]: z.string()
});

export const zDialerWebSocketUpdate = z.strictObject({
  ["headers"]: zWebSocketHeaders.exactOptional()
});

export const zVlessDialerUpdate = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zDialerStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zDialerTls.exactOptional(),
  ["websocket"]: zDialerWebSocketUpdate.nullable().exactOptional()
});

export const zDialerSetUpdate = z.strictObject({
  ["native_tcp"]: zNativeTcpDialer.exactOptional(),
  ["native_udp"]: zNativeUdpDialer.exactOptional(),
  ["trojan"]: zTrojanDialer.exactOptional(),
  ["vless"]: zVlessDialerUpdate.exactOptional()
});

export const zDefaultsUpdate = z.strictObject({
  ["access"]: zAccessDefault.exactOptional(),
  ["dialers"]: zDialerSetUpdate.exactOptional(),
  ["topology"]: zTopologyDefault.exactOptional()
});

export const zNodeSettingsPatch = z.strictObject({
  ["dialers"]: zDialerSetUpdate.nullable().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["exitEnabled"]: z.boolean().exactOptional(),
  ["hostname"]: z.string().exactOptional(),
  ["remark"]: z.string().exactOptional()
});

export const zListenerTls = z.strictObject({
  ["alpn"]: z.array(z.string()).exactOptional(),
  ["certificate"]: zSecretString.exactOptional(),
  ["handshakeTimeout"]: zPositiveDuration.exactOptional(),
  ["maxVersion"]: zTlsVersion.exactOptional(),
  ["minVersion"]: zTlsVersion.exactOptional(),
  ["mode"]: zTlsMode.exactOptional(),
  ["privateKey"]: zSecretString.exactOptional(),
  ["serverName"]: z.string()
});

export const zListenerWebSocketUpdate = z.strictObject({
  ["host"]: z.string().exactOptional(),
  ["path"]: zWebSocketPath.exactOptional(),
  ["responseHeaders"]: zWebSocketHeaders.exactOptional()
});

export const zListenerUpdate = z.union([z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["type"]: z.enum(["native_udp"])
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["type"]: z.enum(["native_tcp"])
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["flow"]: z.string().exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zVlessStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zListenerTls,
  ["type"]: z.enum(["vless"]),
  ["uuid"]: zSecretString.exactOptional(),
  ["websocket"]: zListenerWebSocketUpdate.nullable().exactOptional()
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["password"]: zSecretString.exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zTrojanStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zListenerTls,
  ["type"]: z.enum(["trojan"])
})]);

export const zNodeAddressesInput = z.strictObject({
  ["ipv4"]: z.string().nullable().exactOptional(),
  ["ipv6"]: z.string().nullable().exactOptional()
});

export const zConfigurationOperation = z.union([z.strictObject({
  ["access"]: z.array(zAccessRule),
  ["type"]: z.enum(["replaceAccess"])
}), z.strictObject({
  ["topology"]: z.array(zTopologyRule),
  ["type"]: z.enum(["replaceTopology"])
}), z.strictObject({
  ["routing"]: zRoutingConfiguration,
  ["type"]: z.enum(["replaceRouting"])
}), z.strictObject({
  ["dns"]: zDnsConfiguration,
  ["type"]: z.enum(["replaceDns"])
}), z.strictObject({
  ["defaults"]: zDefaultsUpdate,
  ["type"]: z.enum(["replaceDefaults"])
}), z.strictObject({
  ["groups"]: typedMap(z.string(), z.array(z.string()), {}),
  ["type"]: z.enum(["replaceGroups"])
}), z.strictObject({
  ["tagsOwners"]: typedMap(z.string(), z.array(z.string()), {}),
  ["type"]: z.enum(["replaceTagsOwners"])
}), z.strictObject({
  ["ruleSets"]: typedMap(z.string(), zRuleSet, {}),
  ["type"]: z.enum(["replaceRuleSets"])
}), z.strictObject({
  ["fields"]: zNodeSettingsPatch,
  ["nodeId"]: zNodeId,
  ["type"]: z.enum(["setNodeSettings"])
}), z.strictObject({
  ["key"]: z.string(),
  ["listener"]: zListenerUpdate.nullable(),
  ["nodeId"]: zNodeId,
  ["type"]: z.enum(["setListener"])
}), z.strictObject({
  ["nodeId"]: zNodeId,
  ["tags"]: z.array(z.string()),
  ["type"]: z.enum(["setIdentityTags"])
}), z.strictObject({
  ["addressPools"]: zAddressPoolsInput,
  ["type"]: z.enum(["setAddressPools"])
}), z.strictObject({
  ["addresses"]: zNodeAddressesInput,
  ["nodeId"]: zNodeId,
  ["type"]: z.enum(["setAddresses"])
}), z.strictObject({
  ["nodeId"]: zNodeId,
  ["type"]: z.enum(["deleteNode"])
})]);

export const zListenerChange = z.strictObject({
  ["key"]: z.string(),
  ["kind"]: zChangeKind,
  ["nodeId"]: zNodeId
});

export const zPreparedRuleSet = z.strictObject({
  ["digest"]: zDigest,
  ["entries"]: z.int().gte(0).lte(2147483647)
});

export const zMaterialRequirement = z.enum(["uuid","password","certificate","privateKey","certificateValidation"]);

export const zPreparationKind = z.union([z.enum(["addressPoolV4","addressPoolV6","addressV4","addressV6","identityProof","ruleSetContent"]), z.strictObject({
  ["material"]: zMaterialRequirement
})]);

export const zPreparationRequirement = z.strictObject({
  ["kind"]: zPreparationKind,
  ["path"]: z.string()
});

export const zConfigurationPreviewResponse = z.strictObject({
  ["addressChangedNodeIds"]: z.array(zNodeId),
  ["affectedNodeIds"]: z.array(zNodeId),
  ["baseRevision"]: zRevision,
  ["changeCount"]: z.int().gte(0).lte(2147483647),
  ["changedListeners"]: z.array(zListenerChange),
  ["changes"]: z.array(zConfigurationChange),
  ["deletedNodeIds"]: z.array(zNodeId),
  ["identityChangedNodeIds"]: z.array(zNodeId),
  ["preparedRuleSets"]: typedMap(z.string(), zPreparedRuleSet, {}),
  ["preview"]: zConfigurationView,
  ["requestId"]: zRequestId,
  ["requirements"]: z.array(zPreparationRequirement).exactOptional(),
  ["sourceChanged"]: z.boolean(),
  ["unavailableDependencies"]: z.array(zConfigurationIssue),
  ["warnings"]: z.array(zConfigurationIssue),
  ["willGenerate"]: z.array(zPreparationRequirement).exactOptional()
});

export const zEventId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zStateProblem = z.union([z.strictObject({
  ["code"]: z.enum(["resource_unavailable"]),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["resource"]: zResourceRef
}), z.strictObject({
  ["code"]: z.enum(["configuration_rejected"]),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["target"]: zNetmapRef
}), z.strictObject({
  ["code"]: z.enum(["probe_failed"]),
  ["measurementId"]: zEventId,
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time")
})]);

export const zConfigurationProgress = z.strictObject({
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["phase"]: zApplyPhase,
  ["problem"]: zStateProblem.nullable().exactOptional(),
  ["target"]: zNetmapRef
});

export const zRuleSetBindingChoice = z.enum(["initial","refreshed"]);

export const zConfigurationRequest = z.union([z.strictObject({
  ["expectedRevision"]: zRevision,
  ["requestId"]: zRequestId,
  ["sourceJsonc"]: zWireSecret,
  ["type"]: z.enum(["document"])
}), z.strictObject({
  ["expectedRevision"]: zRevision,
  ["ops"]: z.array(zConfigurationOperation),
  ["requestId"]: zRequestId,
  ["type"]: z.enum(["operations"])
}), z.strictObject({
  ["expectedRevision"]: zRevision,
  ["requestId"]: zRequestId,
  ["restoreFrom"]: zRevision,
  ["ruleSetBindings"]: zRuleSetBindingChoice.nullable().exactOptional(),
  ["type"]: z.enum(["restore"])
})]);

export const zConfigurationResponse = z.strictObject({
  ["configuration"]: zConfigurationView,
  ["networkId"]: zNetworkId,
  ["requestId"]: zRequestId,
  ["revision"]: zRevision
});

export const zConfigurationRevisionSummary = z.strictObject({
  ["cause"]: zConfigurationChangeCause,
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["createdBy"]: zActivityActor,
  ["current"]: z.boolean(),
  ["revision"]: zRevision,
  ["rollbackFrom"]: zRevision.nullable()
});

export const zConfigurationRevisionList = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["currentRevision"]: zRevision,
  ["items"]: z.array(zConfigurationRevisionSummary),
  ["nextCursor"]: z.string().nullable(),
  ["requestId"]: zRequestId
});

export const zConfigurationSchemaResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["schemaJson"]: z.string()
});

export const zDialerSet = z.strictObject({
  ["native_tcp"]: zNativeTcpDialer.exactOptional(),
  ["native_udp"]: zNativeUdpDialer.exactOptional(),
  ["trojan"]: zTrojanDialer.exactOptional(),
  ["vless"]: zVlessDialer.exactOptional()
});

export const zDefaults = z.strictObject({
  ["access"]: zAccessDefault.exactOptional(),
  ["dialers"]: zDialerSet.exactOptional(),
  ["topology"]: zTopologyDefault.exactOptional()
});

export const zNodeSettings = z.strictObject({
  ["addresses"]: zNodeAddresses,
  ["dialers"]: zDialerSet.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["exitEnabled"]: z.boolean().exactOptional(),
  ["hostname"]: z.string(),
  ["identity"]: zNodeIdentity,
  ["listeners"]: typedMap(z.string(), zPreparedListener, {}).exactOptional(),
  ["remark"]: z.string().exactOptional()
});

export const zNetworkConfiguration = z.strictObject({
  ["access"]: z.array(zAccessRule).exactOptional(),
  ["addressPools"]: zAddressPools,
  ["defaults"]: zDefaults.exactOptional(),
  ["dns"]: zDnsConfiguration,
  ["groups"]: typedMap(z.string(), z.array(z.string()), {}).exactOptional(),
  ["nodes"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zNodeSettings, {}).exactOptional(),
  ["routing"]: zRoutingConfiguration.exactOptional(),
  ["ruleSets"]: typedMap(z.string(), zRuleSet, {}).exactOptional(),
  ["tagsOwners"]: typedMap(z.string(), z.array(z.string()), {}).exactOptional(),
  ["topology"]: z.array(zTopologyRule).exactOptional()
});

export const zConfigurationSourceResponse = z.strictObject({
  ["configuration"]: zNetworkConfiguration,
  ["networkId"]: zNetworkId,
  ["requestId"]: zRequestId,
  ["revision"]: zRevision,
  ["sourceJsonc"]: zWireSecret
});

export const zConfigurationState = z.strictObject({
  ["applied"]: zNetmapRef.nullable().exactOptional(),
  ["progress"]: zConfigurationProgress.nullable().exactOptional(),
  ["saved"]: zNetmapRef.nullable().exactOptional()
});

export const zConfigurationValidationParams = z.strictObject({
  ["issues"]: z.array(zConfigurationIssue),
  ["requirements"]: z.array(zPreparationRequirement)
});

export const zCount = z.int().gte(0).lte(9007199254740991);

export const zCoverageReason = z.enum(["retention","capacity"]);

export const zCreateDelegationRequest = z.strictObject({
  ["client"]: z.string(),
  ["codeChallenge"]: z.string(),
  ["codeChallengeMethod"]: zCodeChallengeMethod,
  ["redirectUri"]: z.string()
});

export const zCreateEnrollmentKeyPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListener = z.union([z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["type"]: z.enum(["native_udp"])
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["type"]: z.enum(["native_tcp"])
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["flow"]: z.string().exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zVlessStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zListenerTls,
  ["type"]: z.enum(["vless"]),
  ["uuid"]: zSecretString.exactOptional(),
  ["websocket"]: zListenerWebSocket.exactOptional()
}), z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["password"]: zSecretString.exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zTrojanStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zListenerTls,
  ["type"]: z.enum(["trojan"])
})]);

export const zInitialNodeSettings = z.strictObject({
  ["hostname"]: z.string().nullable().exactOptional(),
  ["listeners"]: typedMap(z.string(), zListener, {})
});

export const zCreateEnrollmentKeyRequest = z.strictObject({
  ["expiresAt"]: zTimestamp,
  ["initialSettings"]: zInitialNodeSettings,
  ["requestId"]: zRequestId,
  ["tags"]: z.array(z.string())
});

export const zExistingNodeBinding = z.strictObject({
  ["keyGeneration"]: zVersion,
  ["keyId"]: zKeyId,
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId
});

export const zEnrollmentIntent = z.strictObject({
  ["codeChallenge"]: z.string(),
  ["controlKey"]: zNodePublicKey,
  ["dataplaneKey"]: zNodePublicKey,
  ["existing"]: zExistingNodeBinding.nullable(),
  ["hostname"]: z.string(),
  ["id"]: zEnrollmentRequestId
});

export const zCreateEnrollmentRequest = z.strictObject({
  ["intent"]: zEnrollmentIntent,
  ["signature"]: zWireSecret
});

export const zCreateJoinLinkPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zCreateJoinLinkRequest = z.strictObject({
  ["expiresInSeconds"]: z.int().gte(60).lte(604800),
  ["requestId"]: zRequestId,
  ["role"]: zMembershipRole
});

export const zCreateNetworkRequest = z.strictObject({
  ["name"]: z.string(),
  ["requestId"]: zRequestId
});

export const zMetricKind = z.enum(["rtt","traffic","probeLoss"]);

export const zObservationPath = z.union([z.strictObject({
  ["mode"]: z.enum(["auto"])
}), z.strictObject({
  ["mode"]: z.enum(["explicit"]),
  ["path"]: z.array(zNodeId)
})]);

export const zObservationServiceScope = z.union([z.strictObject({
  ["port"]: zPort,
  ["type"]: z.enum(["tcp"])
}), z.strictObject({
  ["port"]: zPort,
  ["type"]: z.enum(["udp"])
}), z.strictObject({
  ["type"]: z.enum(["icmpEcho"])
})]);

export const zNodeObservationTarget = z.union([z.strictObject({
  ["name"]: z.string().refine(value => [...value].length <= 253, "maxLength 253 Unicode code points"),
  ["type"]: z.enum(["name"])
}), z.strictObject({
  ["address"]: z.string().refine(value => [...value].length <= 64, "maxLength 64 Unicode code points"),
  ["type"]: z.enum(["meshIp"])
})]);

export const zCreateNodeObservation = z.strictObject({
  ["instance"]: zInstanceRequest,
  ["intervalMs"]: z.int().gte(1000).lte(30000),
  ["metrics"]: z.array(zMetricKind),
  ["selection"]: zObservationPath,
  ["service"]: zObservationServiceScope,
  ["subscriptionId"]: zObservationSubscriptionId,
  ["target"]: zNodeObservationTarget
});

export const zCreateObservation = z.strictObject({
  ["intervalMs"]: z.int().gte(1000).lte(30000),
  ["metrics"]: z.array(zMetricKind),
  ["selection"]: zObservationPath,
  ["service"]: zObservationServiceScope,
  ["sourceNodeId"]: zNodeId,
  ["subscriptionId"]: zObservationSubscriptionId,
  ["targetNodeId"]: zNodeId
});

export const zCreateObservationPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zDelegatedSessionResponse = z.strictObject({
  ["client"]: z.string(),
  ["requestId"]: zRequestId,
  ["server"]: z.string().nullable(),
  ["session"]: zSessionMetadata,
  ["token"]: zWireSecret,
  ["user"]: zUserView
});

export const zDelegationCreated = z.strictObject({
  ["code"]: zWireSecret,
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["requestId"]: zRequestId
});

export const zDeleteNodePath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId
});

export const zDeleteNodeRequest = z.strictObject({
  ["expectedRevision"]: zRevision,
  ["requestId"]: zRequestId
});

export const zDenyDeviceAuthorizationPath = z.strictObject({
  ["enrollmentRequestId"]: zEnrollmentRequestId
});

export const zLookupUnavailableReason = z.enum(["datasetUnavailable","lookupFailed"]);

export const zNotApplicableReason = z.enum(["nonPublicAddress","meshAddress"]);

export const zLookupResult_AsnData = z.union([z.strictObject({
  ["data"]: z.strictObject({
  ["number"]: z.int().gte(0).lte(2147483647),
  ["organization"]: z.string().nullable().exactOptional(),
  ["prefix"]: z.string().nullable().exactOptional()
}),
  ["datasetId"]: zArtifactId,
  ["lookedUpAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["status"]: z.enum(["available"])
}), z.strictObject({
  ["datasetId"]: zArtifactId,
  ["lookedUpAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["status"]: z.enum(["notFound"])
}), z.strictObject({
  ["reason"]: zLookupUnavailableReason,
  ["status"]: z.enum(["unavailable"])
}), z.strictObject({
  ["reason"]: zNotApplicableReason,
  ["status"]: z.enum(["notApplicable"])
})]);

export const zGeoPosition = z.strictObject({
  ["accuracyRadiusKm"]: z.preprocess(unwrapJsonNumber, z.number().gte(0)).nullable().exactOptional(),
  ["latitude"]: z.preprocess(unwrapJsonNumber, z.number().gte(-90).lte(90)),
  ["longitude"]: z.preprocess(unwrapJsonNumber, z.number().gte(-180).lte(180))
});

export const zLookupResult_GeoIpData = z.union([z.strictObject({
  ["data"]: z.strictObject({
  ["city"]: z.string().nullable().exactOptional(),
  ["country"]: z.string().nullable().exactOptional(),
  ["position"]: zGeoPosition.nullable().exactOptional(),
  ["region"]: z.string().nullable().exactOptional()
}),
  ["datasetId"]: zArtifactId,
  ["lookedUpAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["status"]: z.enum(["available"])
}), z.strictObject({
  ["datasetId"]: zArtifactId,
  ["lookedUpAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["status"]: z.enum(["notFound"])
}), z.strictObject({
  ["reason"]: zLookupUnavailableReason,
  ["status"]: z.enum(["unavailable"])
}), z.strictObject({
  ["reason"]: zNotApplicableReason,
  ["status"]: z.enum(["notApplicable"])
})]);

export const zIpInfo = z.strictObject({
  ["asn"]: zLookupResult_AsnData,
  ["geoip"]: zLookupResult_GeoIpData
});

export const zGeoIpData = z.strictObject({
  ["city"]: z.string().nullable().exactOptional(),
  ["country"]: z.string().nullable().exactOptional(),
  ["position"]: zGeoPosition.nullable().exactOptional(),
  ["region"]: z.string().nullable().exactOptional()
});

export const zLocationSource = z.enum(["interface","stun"]);

export const zGeoObservation = z.strictObject({
  ["address"]: z.string(),
  ["data"]: zGeoIpData,
  ["datasetId"]: zArtifactId,
  ["endpointSource"]: zLocationSource,
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["sourceGeneration"]: zSequence,
  ["sourceSocketId"]: z.string()
});

export const zLocationAbsentReason = z.enum(["noObservation","observationExpired","datasetUnavailable","notInDataset","lookupFailed","ambiguousObservation"]);

export const zDerivedNodeState = z.strictObject({
  ["ipInfo"]: typedMap(z.string(), zIpInfo, {}),
  ["location"]: zGeoObservation.nullable().exactOptional(),
  ["locationAbsent"]: zLocationAbsentReason.nullable().exactOptional()
});

export const zEnrollmentApprovalView = z.strictObject({
  ["networkId"]: zNetworkId,
  ["userId"]: zUserId
});

export const zEnrollmentPhase = z.enum(["pending","approved","withdrawn","cancelled","consumed","expired"]);

export const zEnrollmentReturnMethod = z.enum(["poll"]);

export const zEnrollmentRequestView = z.strictObject({
  ["authorizationUrl"]: z.string().nullable(),
  ["controlKey"]: zNodePublicKey,
  ["existing"]: zExistingNodeBinding.nullable(),
  ["expiresAt"]: zTimestamp,
  ["hostname"]: z.string(),
  ["id"]: zEnrollmentRequestId,
  ["phase"]: zEnrollmentPhase,
  ["returnMethod"]: zEnrollmentReturnMethod
});

export const zDeviceAuthorizationView = z.strictObject({
  ["approval"]: zEnrollmentApprovalView.nullable(),
  ["request"]: zEnrollmentRequestView
});

export const zEmptyParams = z.strictObject({

});

export const zEndpointSource = z.union([z.strictObject({
  ["configPath"]: z.string(),
  ["source"]: z.enum(["configured"])
}), z.strictObject({
  ["interface"]: z.string(),
  ["source"]: z.enum(["interface"])
}), z.strictObject({
  ["observer"]: z.string(),
  ["source"]: z.enum(["stun"])
})]);

export const zEndpointCandidate = z.strictObject({
  ["address"]: z.string(),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["origin"]: zEndpointSource
});

export const zEnrollmentDeviceProof = z.strictObject({
  ["signature"]: zWireSecret,
  ["verifier"]: zWireSecret
});

export const zEnrollmentKeyActions = z.strictObject({
  ["revoke"]: z.boolean()
});

export const zEnrollmentKeyId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zInitialNodeSettingsView = z.strictObject({
  ["hostname"]: z.string().nullable(),
  ["listeners"]: typedMap(z.string(), zListenerView, {})
});

export const zEnrollmentKeyView = z.strictObject({
  ["actions"]: zEnrollmentKeyActions,
  ["consumedAt"]: zTimestamp.nullable(),
  ["consumedRequestId"]: zEnrollmentRequestId.nullable().exactOptional(),
  ["createdAt"]: zTimestamp,
  ["createdBy"]: zUserId,
  ["expiresAt"]: zTimestamp,
  ["id"]: zEnrollmentKeyId,
  ["initialSettings"]: zInitialNodeSettingsView,
  ["nodeId"]: zNodeId.nullable(),
  ["revokedAt"]: zTimestamp.nullable(),
  ["tags"]: z.array(z.string())
});

export const zSecretDelivery = z.enum(["available","unavailable"]);

export const zEnrollmentKeyResponse = z.strictObject({
  ["key"]: zEnrollmentKeyView,
  ["secret"]: zWireSecret.nullable().exactOptional(),
  ["secretDelivery"]: zSecretDelivery
});

export const zEnrollmentKeysResponse = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["items"]: z.array(zEnrollmentKeyView),
  ["nextCursor"]: z.string().nullable(),
  ["total"]: zCount
});

export const zEnrollmentPollResponse = z.strictObject({
  ["approval"]: zEnrollmentApprovalView.nullable(),
  ["code"]: zWireSecret.nullable().exactOptional(),
  ["nodeId"]: zNodeId.nullable().exactOptional(),
  ["request"]: zEnrollmentRequestView
});

export const zNodeKeyMetadata = z.strictObject({
  ["control"]: zBoundNodeKey,
  ["dataplane"]: zBoundNodeKey
});

export const zServerSigningKey = z.strictObject({
  ["keyGeneration"]: zVersion,
  ["keyId"]: zKeyId,
  ["publicKey"]: zNodePublicKey
});

export const zEnrollmentResponse = z.strictObject({
  ["controllerId"]: zKeyId,
  ["keys"]: zNodeKeyMetadata,
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId,
  ["serverSigningKey"]: zServerSigningKey.nullable().exactOptional()
});

export const zEntityRemovals = z.strictObject({
  ["members"]: z.array(zMembershipId),
  ["nodes"]: z.array(zNodeId)
});

export const zProblem = z.union([z.strictObject({
  ["code"]: z.enum(["invalid_credentials"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["invalid_current_password"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["invalid_password_reset"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["password_reset_used"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["password_reset_expired"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["password_reset_replaced"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["invalid_session"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["registration_disabled"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["email_taken"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["password_weak"]),
  ["message"]: z.string(),
  ["params"]: zPasswordRules
}), z.strictObject({
  ["code"]: z.enum(["invalid_request"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["request_too_large"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["unsupported_media_type"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["busy"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["service_unavailable"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["not_found"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["method_not_allowed"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["permission_denied"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["request_id_conflict"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["binding_conflict"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["receipt_instance_mismatch"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["receipt_target_mismatch"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["receipt_netmap_superseded"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["receipt_precondition"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["receipt_content_conflict"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["name_conflict"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["invalid_cursor"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["version_conflict"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["last_owner"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["already_member"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["invalid_join_link"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["join_link_expired"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["join_link_used"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["join_link_revoked"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["join_link_issuer_unavailable"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["invalid_delegation"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["revision_conflict"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["configuration_invalid"]),
  ["message"]: z.string(),
  ["params"]: zConfigurationValidationParams
}), z.strictObject({
  ["code"]: z.enum(["state_conflict"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["state_scope_changed"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["state_limit"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
}), z.strictObject({
  ["code"]: z.enum(["observation_limit"]),
  ["message"]: z.string(),
  ["params"]: zEmptyParams
})]);

export const zErrorResponse = z.strictObject({
  ["problem"]: zProblem,
  ["requestId"]: zRequestId.nullable(),
  ["status"]: z.int().gte(0).lte(65535)
});

export const zEvaluateConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zEvaluationConfiguration = z.union([z.strictObject({
  ["type"]: z.enum(["current"])
}), z.strictObject({
  ["expectedRevision"]: zRevision,
  ["sourceJsonc"]: zWireSecret,
  ["type"]: z.enum(["document"])
}), z.strictObject({
  ["expectedRevision"]: zRevision,
  ["ops"]: z.array(zConfigurationOperation),
  ["type"]: z.enum(["operations"])
})]);

export const zEvaluationTarget = z.union([z.strictObject({
  ["nodeId"]: zNodeId,
  ["type"]: z.enum(["node"])
}), z.strictObject({
  ["address"]: z.string(),
  ["type"]: z.enum(["address"])
}), z.strictObject({
  ["name"]: z.string(),
  ["type"]: z.enum(["domain"])
}), z.strictObject({
  ["type"]: z.enum(["internet"])
})]);

export const zEvaluateConfigurationRequest = z.strictObject({
  ["configuration"]: zEvaluationConfiguration,
  ["profile"]: z.string().nullable().exactOptional(),
  ["source"]: zNodeId,
  ["target"]: zEvaluationTarget
});

export const zEvaluatedDestination = z.union([z.strictObject({
  ["nodeId"]: zNodeId,
  ["type"]: z.enum(["node"])
}), z.strictObject({
  ["address"]: z.string(),
  ["class"]: zAddressClass,
  ["type"]: z.enum(["address"])
}), z.strictObject({
  ["name"]: z.string(),
  ["type"]: z.enum(["domain"])
}), z.strictObject({
  ["type"]: z.enum(["internet"])
})]);

export const zEvaluationOutcome = z.union([z.strictObject({
  ["type"]: z.enum(["permitted"])
}), z.strictObject({
  ["type"]: z.enum(["direct"])
}), z.strictObject({
  ["type"]: z.enum(["unmanaged"])
}), z.strictObject({
  ["reason"]: zUnavailableReason,
  ["type"]: z.enum(["unavailable"])
})]);

export const zPlannedEdge = z.strictObject({
  ["a"]: zNodeId,
  ["b"]: zNodeId,
  ["topology"]: z.string().nullable()
});

export const zPlannedPath = z.strictObject({
  ["edges"]: z.array(zPlannedEdge),
  ["nodes"]: z.array(zNodeId)
});

export const zProfileEvaluation = z.strictObject({
  ["allowed"]: z.array(z.string()),
  ["assignment"]: z.string().nullable(),
  ["locked"]: z.boolean(),
  ["name"]: z.string(),
  ["requested"]: z.boolean()
});

export const zRouteKind = z.enum(["mesh","exit","direct","unmanaged"]);

export const zRuleSetMatch = z.strictObject({
  ["entry"]: z.string(),
  ["key"]: z.string()
});

export const zRouteEvaluation = z.strictObject({
  ["kind"]: zRouteKind,
  ["profile"]: z.string().nullable(),
  ["resolutionDependent"]: z.array(z.string()),
  ["rule"]: z.string().nullable(),
  ["ruleSet"]: zRuleSetMatch.nullable(),
  ["targets"]: z.array(zNodeId),
  ["via"]: z.array(zNodeId)
});

export const zExitUnavailableReason = z.enum(["ipForwardDisabled","forwardingProbeFailed","netAdminMissing","nftUnavailable","conntrackUnavailable","reversePathFilterConflict","tunUnavailable","firewallConflict","routeConflict","dnsUnavailable","unsupportedFamily","ownershipConflict"]);

export const zTargetProblem = z.enum(["disabled","exitDisabled","exitUnavailable","unsupportedFamily","notEnrolled","noPath","routingCycle","routingLimit"]);

export const zTargetEvaluation = z.strictObject({
  ["exitUnavailableReason"]: zExitUnavailableReason.nullable().exactOptional(),
  ["nodeId"]: zNodeId,
  ["paths"]: z.int().gte(0).lte(2147483647),
  ["problem"]: zTargetProblem.nullable()
});

export const zEvaluation = z.strictObject({
  ["access"]: zAccessEvaluation,
  ["destination"]: zEvaluatedDestination,
  ["outcome"]: zEvaluationOutcome,
  ["paths"]: z.array(zPlannedPath),
  ["profile"]: zProfileEvaluation,
  ["route"]: zRouteEvaluation.nullable(),
  ["source"]: zNodeId,
  ["targets"]: z.array(zTargetEvaluation)
});

export const zEvaluationResponse = z.strictObject({
  ["draft"]: z.boolean(),
  ["evaluation"]: zEvaluation,
  ["requestId"]: zRequestId,
  ["revision"]: zRevision
});

export const zEventActor = z.union([z.strictObject({
  ["type"]: z.enum(["user"]),
  ["userId"]: zUserId
}), z.strictObject({
  ["nodeId"]: zNodeId,
  ["type"]: z.enum(["node"])
}), z.strictObject({
  ["type"]: z.enum(["system"])
}), z.strictObject({
  ["type"]: z.enum(["redacted"])
})]);

export const zExchangeDelegationRequest = z.strictObject({
  ["client"]: z.string(),
  ["code"]: zWireSecret,
  ["codeVerifier"]: zWireSecret,
  ["redirectUri"]: z.string()
});

export const zExitStatus = z.union([z.strictObject({
  ["status"]: z.enum(["notRequested"])
}), z.strictObject({
  ["status"]: z.enum(["probing"])
}), z.strictObject({
  ["status"]: z.enum(["ready"])
}), z.strictObject({
  ["reason"]: zExitUnavailableReason,
  ["status"]: z.enum(["unavailable"])
})]);

export const zExitCapability = z.strictObject({
  ["installedFamilies"]: z.array(zExitFamily),
  ["instanceId"]: zInstanceId,
  ["netmap"]: zNetmapRef,
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["roleGeneration"]: zDigest,
  ["status"]: zExitStatus,
  ["supportedFamilies"]: z.array(zExitFamily)
});

export const zFreshness = z.enum(["current","awaiting_report","expired","superseded","config_mismatch","revoked"]);

export const zGetConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zGetConfigurationProgressPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["revision"]: zRevision
});

export const zGetConfigurationRevisionPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["revision"]: zRevision
});

export const zGetConfigurationSourcePath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zGetConfigurationSourceQuery = z.strictObject({
  ["revision"]: zRevision.exactOptional()
});

export const zGetDeviceAuthorizationPath = z.strictObject({
  ["enrollmentRequestId"]: zEnrollmentRequestId
});

export const zGetLeaveImpactPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zGetMemberRemovalImpactPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["membershipId"]: zMembershipId
});

export const zGetMemberRoleChangeImpactPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["membershipId"]: zMembershipId,
  ["role"]: zMembershipRole
});

export const zGetNetworkActivityPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["activityId"]: zActivityId
});

export const zGetNetworkPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zGetNetworkStatePath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zGetNodeObservationPath = z.strictObject({
  ["subscriptionId"]: zObservationSubscriptionId
});

export const zGetNodeObservationQuery = z.strictObject({
  ["generation"]: zVersion,
  ["instanceId"]: zInstanceId
});

export const zGetNodePath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId
});

export const zGetNodeSettingsPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId
});

export const zGetNodeStatePath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId
});

export const zGetObservationPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["subscriptionId"]: zObservationSubscriptionId
});

export const zGetRuleSetPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["ruleSetKey"]: z.string()
});

export const zLinkAcquisition = z.union([z.strictObject({
  ["type"]: z.enum(["publishedEndpoint"])
}), z.strictObject({
  ["type"]: z.enum(["localEndpoint"])
}), z.strictObject({
  ["type"]: z.enum(["stunEndpoint"])
}), z.strictObject({
  ["type"]: z.enum(["acceptedInbound"])
}), z.strictObject({
  ["attemptId"]: zUdpTraversalId,
  ["endpointSource"]: zEndpointKind,
  ["type"]: z.enum(["coordinatedUdp"])
})]);

export const zSocketAddress = z.string().refine(socketAddress, "Expected numeric IP:port without an IPv6 scope");

export const zProbeChannel = z.strictObject({
  ["acquisition"]: zLinkAcquisition,
  ["linkId"]: zDigest,
  ["localAddress"]: zSocketAddress,
  ["peerNodeId"]: zNodeId,
  ["remoteAddress"]: zSocketAddress,
  ["socketId"]: zDigest,
  ["transport"]: zTransportProtocol
});

export const zProbeChannels = z.strictObject({
  ["reply"]: zProbeChannel.nullable(),
  ["request"]: zProbeChannel
});

export const zProbeKind = z.enum(["neighbor","path","connect","controller"]);

export const zProbeOutcome = z.enum(["pending","success","timeout","failed"]);

export const zProbeObservation = z.strictObject({
  ["channels"]: zProbeChannels.nullable().exactOptional(),
  ["completedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["durationUs"]: zSequence.nullable().exactOptional(),
  ["kind"]: zProbeKind,
  ["linkId"]: z.string().nullable().exactOptional(),
  ["measurementId"]: zEventId,
  ["netmap"]: zNetmapRef,
  ["outcome"]: zProbeOutcome,
  ["path"]: z.array(zNodeId),
  ["startedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["transport"]: z.string().nullable().exactOptional()
});

export const zObservationPayload = z.union([z.strictObject({
  ["attemptId"]: z.string(),
  ["kind"]: z.enum(["dial_started"]),
  ["peerNodeId"]: zNodeId,
  ["transport"]: z.string()
}), z.strictObject({
  ["attemptId"]: z.string(),
  ["durationUs"]: zSequence.nullable().exactOptional(),
  ["kind"]: z.enum(["dial_failed"]),
  ["peerNodeId"]: zNodeId,
  ["reason"]: zCloseReason,
  ["transport"]: z.string()
}), z.strictObject({
  ["kind"]: z.enum(["link_opened"]),
  ["linkId"]: z.string(),
  ["peerNodeId"]: zNodeId,
  ["transport"]: z.string()
}), z.strictObject({
  ["kind"]: z.enum(["link_closed"]),
  ["linkId"]: z.string(),
  ["peerNodeId"]: zNodeId,
  ["reason"]: zCloseReason,
  ["transport"]: z.string()
}), z.strictObject({
  ["kind"]: z.enum(["probe_completed"]),
  ["probe"]: zProbeObservation
}), z.strictObject({
  ["dropped"]: zSequence,
  ["from"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["kind"]: z.enum(["collection_gap"]),
  ["to"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time")
})]);

export const zObservationEvent = z.strictObject({
  ["eventId"]: zEventId,
  ["netmap"]: zNetmapRef,
  ["occurredAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["payload"]: zObservationPayload
});

export const zHistoricalEvent = z.strictObject({
  ["event"]: zObservationEvent,
  ["instanceId"]: zInstanceId,
  ["receivedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["sourceNodeId"]: zNodeId
});

export const zHistoryAccepted = z.strictObject({
  ["accepted"]: z.int().gte(0).lte(2147483647),
  ["duplicates"]: z.int().gte(0).lte(2147483647)
});

export const zHistoryBucket = z.strictObject({
  ["failed"]: z.int().gte(0).lte(2147483647),
  ["from"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["instanceId"]: zInstanceId,
  ["kind"]: zProbeKind,
  ["linkId"]: z.string().nullable().exactOptional(),
  ["meanSuccessDurationUs"]: z.preprocess(unwrapJsonNumber, z.number()).nullable().exactOptional(),
  ["netmap"]: zNetmapRef,
  ["path"]: z.array(zNodeId),
  ["sampleCount"]: z.int().gte(0).lte(2147483647),
  ["sourceNodeId"]: zNodeId,
  ["success"]: z.int().gte(0).lte(2147483647),
  ["timeout"]: z.int().gte(0).lte(2147483647),
  ["to"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["transport"]: z.string().nullable().exactOptional()
});

export const zHistoryCoverage = z.union([z.strictObject({
  ["before"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["reason"]: zCoverageReason,
  ["sourceNodeId"]: zNodeId,
  ["type"]: z.enum(["pruned"])
}), z.strictObject({
  ["dropped"]: zSequence,
  ["eventId"]: zEventId,
  ["from"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["instanceId"]: zInstanceId,
  ["sourceNodeId"]: zNodeId,
  ["to"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["type"]: z.enum(["collectionGap"])
})]);

export const zRecentLink = z.strictObject({
  ["closed"]: z.boolean(),
  ["lastEventAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["linkId"]: z.string(),
  ["peerNodeId"]: zNodeId,
  ["sourceNodeId"]: zNodeId,
  ["transport"]: z.string()
});

export const zHistoryItems = z.union([z.strictObject({
  ["items"]: z.array(zHistoricalEvent),
  ["mode"]: z.enum(["raw"])
}), z.strictObject({
  ["items"]: z.array(zHistoryBucket),
  ["mode"]: z.enum(["buckets"])
}), z.strictObject({
  ["items"]: z.array(zRecentLink),
  ["mode"]: z.enum(["recentLinks"])
})]);

export const zHistoryKind = z.enum(["dial_started","dial_failed","link_opened","link_closed","probe_completed","collection_gap"]);

export const zHistoryMode = z.enum(["raw","buckets","recentLinks"]);

export const zHistoryPage = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["coverage"]: z.array(zHistoryCoverage),
  ["from"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["nextCursor"]: z.string().nullable().exactOptional(),
  ["result"]: zHistoryItems,
  ["to"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time")
});

export const zHistoryQuery = z.strictObject({
  ["bucketSeconds"]: z.int().gte(0).lte(2147483647).nullable().exactOptional(),
  ["cursor"]: z.string().nullable().exactOptional(),
  ["from"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["kinds"]: z.array(zHistoryKind).nullable().exactOptional(),
  ["limit"]: z.int().gte(1).lte(200),
  ["linkId"]: z.string().nullable().exactOptional(),
  ["mode"]: zHistoryMode,
  ["peerNodeId"]: zNodeId.nullable().exactOptional(),
  ["sourceNodeId"]: zNodeId.nullable().exactOptional(),
  ["to"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time")
});

export const zHistoryReport = z.strictObject({
  ["events"]: z.array(zObservationEvent),
  ["instanceId"]: zInstanceId
});

export const zInstanceLease = z.strictObject({
  ["expiresAt"]: zTimestamp,
  ["generation"]: zVersion,
  ["instanceId"]: zInstanceId
});

export const zJoinLinkActions = z.strictObject({
  ["revoke"]: z.boolean()
});

export const zLinkUser = z.strictObject({
  ["email"]: z.string(),
  ["userId"]: zUserId
});

export const zJoinLinkView = z.strictObject({
  ["actions"]: zJoinLinkActions,
  ["consumedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable(),
  ["consumedBy"]: zLinkUser.nullable(),
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["id"]: zJoinLinkId,
  ["issuedBy"]: zLinkUser,
  ["networkId"]: zNetworkId,
  ["revokedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable(),
  ["role"]: zMembershipRole
});

export const zJoinLinkCreated = z.union([z.strictObject({
  ["link"]: zJoinLinkView,
  ["requestId"]: zRequestId,
  ["secret"]: zWireSecret,
  ["secretDelivery"]: z.enum(["available"])
}), z.strictObject({
  ["link"]: zJoinLinkView,
  ["requestId"]: zRequestId,
  ["secretDelivery"]: z.enum(["unavailable"])
})]);

export const zJoinLinkCredentialRequest = z.strictObject({
  ["token"]: zWireSecret
});

export const zJoinLinkInspection = z.strictObject({
  ["alreadyMember"]: z.boolean(),
  ["link"]: zJoinLinkView,
  ["networkName"]: z.string(),
  ["requestId"]: zRequestId
});

export const zJoinLinkListResponse = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["items"]: z.array(zJoinLinkView),
  ["nextCursor"]: z.string().nullable(),
  ["requestId"]: zRequestId,
  ["total"]: zCount
});

export const zJoinLinkRedeemed = z.strictObject({
  ["membershipId"]: zMembershipId,
  ["networkId"]: zNetworkId,
  ["requestId"]: zRequestId
});

export const zKeyEnrollmentRequest = z.strictObject({
  ["enrollmentKey"]: zWireSecret,
  ["request"]: zCreateEnrollmentRequest,
  ["verifier"]: zWireSecret
});

export const zLeaveNetworkPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zObservationId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zLinkState = z.strictObject({
  ["acquisition"]: zLinkAcquisition.nullable().exactOptional(),
  ["businessDependencies"]: z.int().gte(0).lte(2147483647),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["initiatorNodeId"]: zNodeId,
  ["linkId"]: z.string(),
  ["observationIds"]: z.array(zObservationId),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["phase"]: zLinkPhase,
  ["since"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["socketId"]: z.string(),
  ["transport"]: z.string()
});

export const zListAccountActivityQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListAccountSessionsQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListConfigurationRevisionsPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListConfigurationRevisionsQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListEnrollmentKeysPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListEnrollmentKeysQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListJoinLinksPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListJoinLinksQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListNetworkActivityPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListNetworkActivityQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListNetworkMembersPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListNetworkMembersQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListNetworksQuery = z.strictObject({
  ["cursor"]: z.string().refine(value => [...value].length <= 4096, "maxLength 4096 Unicode code points").exactOptional(),
  ["limit"]: z.int().gte(1).lte(200).exactOptional()
});

export const zListNodesPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListObservationsPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListRuleSetsPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zListenerPhase = z.enum(["preparing","listening","stopped","failed"]);

export const zListenerState = z.strictObject({
  ["loadedCertificateSha256"]: zDigest.nullable().exactOptional(),
  ["netmap"]: zNetmapRef.nullable().exactOptional(),
  ["phase"]: zListenerPhase,
  ["problem"]: zStateProblem.nullable().exactOptional(),
  ["socketIds"]: z.array(z.string())
});

export const zLocalEndpointSource = z.enum(["local"]);

export const zLocalEndpoint = z.strictObject({
  ["address"]: z.string(),
  ["source"]: zLocalEndpointSource
});

export const zLoginRequest = z.strictObject({
  ["email"]: z.string(),
  ["password"]: zWireSecret
});

export const zMemberActions = z.strictObject({
  ["assignableRoles"]: z.array(zMembershipRole),
  ["changeRole"]: z.boolean(),
  ["remove"]: z.boolean()
});

export const zMemberSummary = z.strictObject({
  ["actions"]: zMemberActions,
  ["email"]: z.string(),
  ["joinedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["membershipId"]: zMembershipId,
  ["role"]: zMembershipRole,
  ["userId"]: zUserId,
  ["version"]: zVersion
});

export const zMemberMutationResponse = z.strictObject({
  ["member"]: zMemberSummary,
  ["requestId"]: zRequestId
});

export const zMemberRemovedResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["revision"]: zRevision
});

export const zNativeTcpListener = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional()
});

export const zNativeUdpListener = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional()
});

export const zNetmapDialerDelta = z.strictObject({
  ["remove"]: z.array(z.string()),
  ["upsert"]: typedMap(z.string(), zDialCandidate, {})
});

export const zNetmapInventoryDelta = z.strictObject({
  ["remove"]: z.array(zNodeId),
  ["upsert"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zTrustedNode, {})
});

export const zNetmapNodeDelta = z.strictObject({
  ["remove"]: z.array(zNodeId),
  ["upsert"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zNodeSettingsView, {})
});

export const zNetmapDelta = z.strictObject({
  ["base"]: zNetmapRef,
  ["configuration"]: zConfigurationView,
  ["dependencyDigest"]: zDigest,
  ["dialers"]: zNetmapDialerDelta,
  ["dns"]: zNetmapDns,
  ["egress"]: zEgressPlan.nullable(),
  ["exitRole"]: zExitRole.nullable(),
  ["inventory"]: zNetmapInventoryDelta,
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId,
  ["nodes"]: zNetmapNodeDelta,
  ["ruleSets"]: typedMap(z.string(), zRuleSetContent, {}),
  ["selectedProfiles"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), z.string(), {}),
  ["selectionVersions"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zVersion, {}),
  ["self"]: zNetmapSelf,
  ["subjects"]: zSubjectContext,
  ["target"]: zNetmapRef
});

export const zNetmapLease = z.strictObject({
  ["expiresAt"]: zTimestamp,
  ["issuedAt"]: zTimestamp,
  ["netmap"]: zNetmapRef
});

export const zNetmapSignature = z.strictObject({
  ["keyGeneration"]: zVersion,
  ["keyId"]: zKeyId,
  ["signature"]: z.string()
});

export const zNetworkActions = z.strictObject({
  ["editConfiguration"]: z.boolean(),
  ["joinLinkRoles"]: z.array(zMembershipRole),
  ["manageMembers"]: z.boolean(),
  ["readConfiguration"]: z.boolean(),
  ["rename"]: z.boolean()
});

export const zNetworkSummary = z.strictObject({
  ["actions"]: zNetworkActions,
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["currentRevision"]: zRevision,
  ["id"]: zNetworkId,
  ["membershipId"]: zMembershipId,
  ["name"]: z.string(),
  ["role"]: zMembershipRole
});

export const zNetworkListResponse = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["items"]: z.array(zNetworkSummary),
  ["nextCursor"]: z.string().nullable(),
  ["requestId"]: zRequestId,
  ["total"]: zCount
});

export const zNetworkMembersResponse = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["items"]: z.array(zMemberSummary),
  ["nextCursor"]: z.string().nullable(),
  ["requestId"]: zRequestId,
  ["total"]: zCount
});

export const zNetworkResponse = z.strictObject({
  ["network"]: zNetworkSummary,
  ["requestId"]: zRequestId
});

export const zProofOperation = z.enum(["session","rotate"]);

export const zNodeChallenge = z.strictObject({
  ["controllerId"]: zKeyId,
  ["expiresAt"]: zTimestamp,
  ["id"]: zChallengeId,
  ["keyGeneration"]: zVersion,
  ["keyId"]: zKeyId,
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId,
  ["nonce"]: zDigest,
  ["operation"]: zProofOperation,
  ["requestDigest"]: zDigest
});

export const zNodeChallengeRequest = z.strictObject({
  ["keyId"]: zKeyId,
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId,
  ["operation"]: zProofOperation,
  ["requestDigest"]: zDigest
});

export const zNodeChallengeIntent = z.strictObject({
  ["controllerId"]: zKeyId,
  ["expiresAt"]: zTimestamp,
  ["id"]: zChallengeId,
  ["request"]: zNodeChallengeRequest
});

export const zNodeInventoryView = z.strictObject({
  ["addresses"]: zNodeAddresses,
  ["canDelete"]: z.boolean(),
  ["canRevokeControlKey"]: z.boolean(),
  ["canRevokeDataplaneKey"]: z.boolean(),
  ["controlKeyCreatedAt"]: zTimestamp,
  ["controlKeyRevokedAt"]: zTimestamp.nullable(),
  ["dataplaneKeyCreatedAt"]: zTimestamp,
  ["dataplaneKeyRevokedAt"]: zTimestamp.nullable(),
  ["enabled"]: z.boolean(),
  ["hostname"]: z.string(),
  ["id"]: zNodeId,
  ["identity"]: zNodeIdentity,
  ["keys"]: zNodeKeyMetadata,
  ["netmap"]: zNetmapRef.nullable(),
  ["networkId"]: zNetworkId,
  ["registeredAt"]: zTimestamp
});

export const zNodeInventoryResponse = z.strictObject({
  ["items"]: z.array(zNodeInventoryView)
});

export const zProbeWindow = z.strictObject({
  ["pending"]: z.int().gte(0).lte(2147483647),
  ["repliedWithinDeadline"]: z.int().gte(0).lte(2147483647),
  ["sent"]: z.int().gte(0).lte(2147483647),
  ["timeout"]: z.int().gte(0).lte(2147483647)
});

export const zTrafficWindow = z.strictObject({
  ["counterGeneration"]: zSequence,
  ["rxBytes"]: zSequence,
  ["txBytes"]: zSequence
});

export const zObservationSample = z.strictObject({
  ["measurementId"]: zEventId,
  ["partial"]: z.boolean(),
  ["probe"]: zProbeObservation.nullable().exactOptional(),
  ["probeCounts"]: zProbeWindow.nullable().exactOptional(),
  ["traffic"]: zTrafficWindow.nullable().exactOptional(),
  ["version"]: zVersion,
  ["windowEnd"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["windowStart"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time")
});

export const zObservationPhase = z.enum(["preparing","running","completed","failed","expired","cancelled","unsupported"]);

export const zObservationTask = z.strictObject({
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["id"]: zObservationId,
  ["instanceId"]: zInstanceId,
  ["intervalMs"]: z.int().gte(0).lte(2147483647),
  ["latest"]: zObservationSample.nullable().exactOptional(),
  ["metrics"]: z.array(zMetricKind),
  ["netmap"]: zNetmapRef,
  ["path"]: z.array(zNodeId).nullable().exactOptional(),
  ["pathSegment"]: zSequence.nullable().exactOptional(),
  ["phase"]: zObservationPhase,
  ["renewAfter"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["selection"]: zObservationPath,
  ["service"]: zObservationServiceScope,
  ["sourceNodeId"]: zNodeId,
  ["subscriberExpiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["subscriptionId"]: zObservationSubscriptionId.nullable().exactOptional(),
  ["targetNodeId"]: zNodeId,
  ["unsupportedMetrics"]: z.array(zMetricKind)
});

export const zNodeObservationCancelled = z.strictObject({
  ["task"]: zObservationTask.nullable()
});

export const zNodeObservationQuery = z.strictObject({
  ["generation"]: zVersion,
  ["instanceId"]: zInstanceId
});

export const zNodeObservationTargetView = z.strictObject({
  ["addresses"]: zNodeAddresses,
  ["hostname"]: z.string(),
  ["nodeId"]: zNodeId
});

export const zNodeObservationResponse = z.strictObject({
  ["target"]: zNodeObservationTargetView,
  ["task"]: zObservationTask
});

export const zNodeProof = z.strictObject({
  ["challengeId"]: zChallengeId,
  ["signature"]: zWireSecret
});

export const zNodeRotationIntent = z.strictObject({
  ["controlKey"]: zNodePublicKey,
  ["dataplaneKey"]: zNodePublicKey,
  ["requestId"]: zRequestId
});

export const zNodeRotationRequest = z.strictObject({
  ["intent"]: zNodeRotationIntent,
  ["newSignature"]: zWireSecret,
  ["oldProof"]: zNodeProof
});

export const zNodeSessionIntent = z.strictObject({
  ["requestId"]: zRequestId
});

export const zNodeSessionRequest = z.strictObject({
  ["intent"]: zNodeSessionIntent,
  ["proof"]: zNodeProof
});

export const zNodeSessionResponse = z.strictObject({
  ["expiresAt"]: zTimestamp,
  ["keyId"]: zKeyId,
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId,
  ["token"]: zWireSecret
});

export const zNodeSettingsResponse = z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId,
  ["requestId"]: zRequestId,
  ["revision"]: zRevision,
  ["settings"]: zNodeSettingsView
});

export const zSelectionState = z.strictObject({
  ["appliedProfile"]: z.string().nullable().exactOptional()
});

export const zSocketNetwork = z.enum(["udp","tcp"]);

export const zSocketState = z.strictObject({
  ["boundAddress"]: z.string(),
  ["endpoints"]: z.array(zEndpointCandidate),
  ["generation"]: zSequence,
  ["localEndpoints"]: z.array(zLocalEndpoint).max(8).nullable().exactOptional(),
  ["netmap"]: zNetmapRef.nullable().exactOptional(),
  ["network"]: zSocketNetwork,
  ["resource"]: zResourceRef
});

export const zSystemState = z.strictObject({
  ["agentVersion"]: z.string(),
  ["arch"]: z.string(),
  ["hostname"]: z.string(),
  ["os"]: z.string()
});

export const zNodeState = z.strictObject({
  ["capabilities"]: typedMap(z.string(), zCapability, {}),
  ["configuration"]: zConfigurationState,
  ["exitCapability"]: zExitCapability.nullable().exactOptional(),
  ["links"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), z.array(zLinkState), {}),
  ["listeners"]: typedMap(z.string(), zListenerState, {}),
  ["paths"]: typedMap(z.string(), zPathState, {}),
  ["probes"]: z.array(zProbeObservation),
  ["problems"]: z.array(zStateProblem),
  ["selection"]: zSelectionState,
  ["sockets"]: typedMap(z.string(), zSocketState, {}),
  ["system"]: zSystemState
});

export const zPeerCandidate = z.strictObject({
  ["address"]: z.string(),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["source"]: zCandidateSource
});

export const zPeerEndpoint = z.strictObject({
  ["candidate"]: zPeerCandidate,
  ["candidateReference"]: zCandidateReference,
  ["generation"]: zSequence,
  ["network"]: zSocketNetwork,
  ["resource"]: zResourceRef,
  ["socketId"]: z.string()
});

export const zPeerState = z.strictObject({
  ["capabilities"]: typedMap(z.string(), zCapability, {}),
  ["configuration"]: zConfigurationState,
  ["endpoints"]: z.array(zPeerEndpoint)
});

export const zVisibleReport = z.union([z.strictObject({
  ["report"]: zNodeState,
  ["view"]: z.enum(["full"])
}), z.strictObject({
  ["report"]: zPeerState,
  ["view"]: z.enum(["peer"])
})]);

export const zNodeStateView = z.strictObject({
  ["derived"]: zDerivedNodeState,
  ["freshness"]: zFreshness,
  ["hostname"]: z.string(),
  ["instanceId"]: zInstanceId.nullable().exactOptional(),
  ["lastAuthenticatedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["receivedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["report"]: zVisibleReport.nullable().exactOptional(),
  ["reportExpiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["reportSequence"]: zSequence.nullable().exactOptional()
});

export const zObservationList = z.strictObject({
  ["items"]: z.array(zObservationTask)
});

export const zReportedObservationPhase = z.enum(["preparing","failed","completed","unsupported"]);

export const zObservationUpdate = z.union([z.strictObject({
  ["phase"]: zReportedObservationPhase,
  ["type"]: z.enum(["phase"])
}), z.strictObject({
  ["path"]: z.array(zNodeId),
  ["pathSegment"]: zSequence,
  ["sample"]: zObservationSample,
  ["type"]: z.enum(["sample"])
})]);

export const zObservationResult = z.strictObject({
  ["instanceId"]: zInstanceId,
  ["netmap"]: zNetmapRef,
  ["result"]: zObservationUpdate
});

export const zOuterSocket = z.enum(["udp","tcp"]);

export const zPasswordResetInspectRequest = z.strictObject({
  ["token"]: zWireSecret
});

export const zPasswordResetInspection = z.strictObject({
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["requestId"]: zRequestId
});

export const zPasswordResetRedeemRequest = z.strictObject({
  ["password"]: zWireSecret,
  ["token"]: zWireSecret
});

export const zPeerExitFacts = z.strictObject({
  ["installedFamilies"]: z.array(zExitFamily),
  ["roleGeneration"]: zDigest,
  ["status"]: zExitStatus
});

export const zPeerListenerFacts = z.strictObject({
  ["localEndpoints"]: z.array(zLocalEndpoint),
  ["network"]: zSocketNetwork,
  ["phase"]: zListenerPhase,
  ["stunCandidates"]: z.array(zEndpointCandidate)
});

export const zPeerFacts = z.strictObject({
  ["exit"]: zPeerExitFacts.nullable(),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["listeners"]: typedMap(z.string(), zPeerListenerFacts, {}),
  ["nodeId"]: zNodeId,
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time")
});

export const zStateCursor = z.strictObject({
  ["epoch"]: zStateEpoch,
  ["sequence"]: zSequence
});

export const zPeerFactsDelta = z.strictObject({
  ["base"]: zStateCursor.nullable(),
  ["cursor"]: zStateCursor,
  ["netmapSerial"]: zVersion,
  ["removed"]: z.array(zNodeId),
  ["updated"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zPeerFacts, {})
});

export const zPrepareUdpTraversalRequest = z.strictObject({
  ["carrier"]: zCarrier,
  ["expiresAt"]: zTimestamp,
  ["instance"]: zInstanceRequest,
  ["localCandidate"]: zCandidateReference,
  ["netmap"]: zNetmapRef,
  ["peerCandidate"]: zCandidateReference,
  ["peerId"]: zNodeId,
  ["requestId"]: zRequestId,
  ["stateEpoch"]: zStateEpoch
});

export const zPrepareUdpTraversalResponse = z.union([z.strictObject({
  ["offer"]: zUdpTraversalOffer,
  ["status"]: z.enum(["offer"])
}), z.strictObject({
  ["reason"]: zUdpTraversalUnavailableReason,
  ["status"]: z.enum(["unavailable"])
})]);

export const zQueryEnrollmentRequestPath = z.strictObject({
  ["enrollmentRequestId"]: zEnrollmentRequestId
});

export const zQueryObservationHistoryPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zQueryUdpTraversalRequest = z.strictObject({
  ["instance"]: zInstanceRequest,
  ["netmap"]: zNetmapRef
});

export const zUdpTraversalPhase = z.enum(["waiting","active"]);

export const zUdpTraversalAttempts = z.array(z.strictObject({
  ["offer"]: zUdpTraversalOffer,
  ["phase"]: zUdpTraversalPhase
})).max(32);

export const zStunServers = z.array(z.strictObject({
  ["endpoint"]: zEndpoint,
  ["id"]: z.string().refine(value => [...value].length >= 1, "minLength 1 Unicode code points").refine(value => [...value].length <= 64, "maxLength 64 Unicode code points").regex(new RegExp("^[A-Za-z0-9_-]{1,64}(?![\\s\\S])", "u"))
})).max(4);

export const zQueryUdpTraversalResponse = z.union([z.strictObject({
  ["asOf"]: zTimestamp,
  ["attempts"]: zUdpTraversalAttempts,
  ["expiresAt"]: zTimestamp,
  ["stateEpoch"]: zStateEpoch,
  ["status"]: z.enum(["current"]),
  ["stunServers"]: zStunServers
}), z.strictObject({
  ["reason"]: zUdpTraversalUnavailableReason,
  ["status"]: z.enum(["unavailable"])
})]);

export const zReceiptPhase = z.enum(["saved","applied","failed"]);

export const zReceiptRequest = z.strictObject({
  ["attempt"]: zVersion,
  ["failure"]: zApplyFailure.nullable().exactOptional(),
  ["instance"]: zInstanceRequest,
  ["netmap"]: zNetmapRef,
  ["phase"]: zReceiptPhase,
  ["reportedAt"]: zTimestamp,
  ["sequence"]: zSequence
});

export const zRedeemEnrollmentRequest = z.strictObject({
  ["code"]: zWireSecret,
  ["proof"]: zEnrollmentDeviceProof
});

export const zRedeemEnrollmentRequestPath = z.strictObject({
  ["enrollmentRequestId"]: zEnrollmentRequestId
});

export const zRefreshRuleSetPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["ruleSetKey"]: z.string()
});

export const zRefreshRuleSetRequest = z.strictObject({
  ["requestId"]: zRequestId
});

export const zRegisterRequest = z.strictObject({
  ["email"]: z.string(),
  ["password"]: zWireSecret
});

export const zRemovalImpactResponse = z.strictObject({
  ["membershipId"]: zMembershipId,
  ["nodeCount"]: z.int().gte(0).lte(2147483647),
  ["nodeIds"]: z.array(zNodeId),
  ["requestId"]: zRequestId,
  ["revision"]: zRevision,
  ["version"]: zVersion
});

export const zRemoveMemberPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["membershipId"]: zMembershipId
});

export const zRemoveMemberRequest = z.strictObject({
  ["expectedRevision"]: zRevision,
  ["expectedVersion"]: zVersion
});

export const zRenameNetworkPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zRenameNetworkRequest = z.strictObject({
  ["expectedName"]: z.string(),
  ["name"]: z.string(),
  ["requestId"]: zRequestId
});

export const zRenewNodeObservation = z.strictObject({
  ["expiresAt"]: zTimestamp,
  ["instance"]: zInstanceRequest
});

export const zRenewNodeObservationPath = z.strictObject({
  ["subscriptionId"]: zObservationSubscriptionId
});

export const zRenewObservationPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["subscriptionId"]: zObservationSubscriptionId
});

export const zReportTaskResultPath = z.strictObject({
  ["taskId"]: zObservationId
});

export const zResetReason = z.enum(["cursor_too_old","view_changed","backpressure","epoch_changed"]);

export const zResourceProblem = z.enum(["download_failed","invalid_source","too_large","conversion_failed","configuration_changed","budget_exceeded","not_ready","validation_failed"]);

export const zRevokeAccountSessionPath = z.strictObject({
  ["sessionId"]: zSessionId
});

export const zRevokeEnrollmentKeyPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["keyId"]: zEnrollmentKeyId
});

export const zRevokeJoinLinkPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["linkId"]: zJoinLinkId
});

export const zRevokeNodeKeyPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodeId"]: zNodeId
});

export const zRevokeNodeKeyRequest = z.strictObject({
  ["expectedGeneration"]: zVersion,
  ["keyId"]: zKeyId
});

export const zRoleChangeImpactResponse = z.strictObject({
  ["joinLinkCount"]: z.int().gte(0).lte(2147483647),
  ["joinLinks"]: z.array(zJoinLinkView),
  ["membershipId"]: zMembershipId,
  ["requestId"]: zRequestId,
  ["role"]: zMembershipRole,
  ["version"]: zVersion
});

export const zRoutingSelectionRequest = z.strictObject({
  ["expectedVersion"]: zVersion.nullable().exactOptional(),
  ["instance"]: zInstanceRequest,
  ["profile"]: z.string()
});

export const zRoutingSelectionResponse = z.strictObject({
  ["profile"]: z.string(),
  ["version"]: zVersion
});

export const zRuleSetView = z.strictObject({
  ["artifact"]: zArtifactSummary.nullable().exactOptional(),
  ["checkedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["configurationRevision"]: zRevision,
  ["definition"]: zRuleSet.nullable().exactOptional(),
  ["key"]: z.string(),
  ["lastSuccessAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["problem"]: zResourceProblem.nullable().exactOptional()
});

export const zRuleSetList = z.strictObject({
  ["items"]: z.array(zRuleSetView)
});

export const zSessionIssuedResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["session"]: zSessionMetadata,
  ["token"]: zWireSecret,
  ["user"]: zUserView
});

export const zSignedNodeChallengeRequest = z.strictObject({
  ["intent"]: zNodeChallengeIntent,
  ["signature"]: zWireSecret
});

export const zStateAccepted = z.strictObject({
  ["duplicate"]: z.boolean(),
  ["receivedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["sequence"]: zSequence
});

export const zStateQuery = z.strictObject({
  ["nodes"]: z.array(zNodeId).nullable().exactOptional()
});

export const zStateReport = z.strictObject({
  ["instanceId"]: zInstanceId,
  ["observedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["sequence"]: zSequence,
  ["state"]: zNodeState
});

export const zStateView = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["configurationRevision"]: zRevision,
  ["links"]: z.array(zAggregatedLink),
  ["networkId"]: zNetworkId,
  ["nodes"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zNodeStateView, {}),
  ["paths"]: z.array(zAggregatedPath),
  ["sequence"]: zSequence,
  ["stateEpoch"]: zStateEpoch
});

export const zSyncNetmap = z.union([z.strictObject({
  ["netmap"]: zCompiledNetmap,
  ["type"]: z.enum(["full"])
}), z.strictObject({
  ["delta"]: zNetmapDelta,
  ["type"]: z.enum(["delta"])
}), z.strictObject({
  ["type"]: z.enum(["unchanged"])
})]);

export const zSyncRequest = z.strictObject({
  ["netmap"]: zNetmapRef.nullable().exactOptional(),
  ["state"]: zStateCursor.nullable().exactOptional()
});

export const zSyncResponse = z.strictObject({
  ["lease"]: zNetmapLease,
  ["netmap"]: zSyncNetmap,
  ["peerFacts"]: zPeerFactsDelta.nullable(),
  ["signature"]: zNetmapSignature
});

export const zTransportCatalogEntry = z.strictObject({
  ["dialerSchema"]: z.string(),
  ["listenerSchema"]: z.string(),
  ["outerSocket"]: zOuterSocket,
  ["protocol"]: zTransportProtocol,
  ["requiredCapability"]: z.string()
});

export const zTransportCatalogResponse = z.strictObject({
  ["items"]: z.array(zTransportCatalogEntry),
  ["requestId"]: zRequestId
});

export const zTrojanListener = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["password"]: zSecretString.exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zTrojanStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zListenerTls
});

export const zUdpTraversalAttemptRequest = z.strictObject({
  ["digest"]: zDigest,
  ["instance"]: zInstanceRequest,
  ["netmap"]: zNetmapRef,
  ["stateEpoch"]: zStateEpoch,
  ["version"]: zVersion
});

export const zUserConfigurationView = z.union([z.strictObject({
  ["configuration"]: zConfigurationView,
  ["networkId"]: zNetworkId,
  ["revision"]: zRevision,
  ["type"]: z.enum(["manager"])
}), z.strictObject({
  ["networkId"]: zNetworkId,
  ["nodes"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zNodeSettingsView, {}),
  ["revision"]: zRevision,
  ["type"]: z.enum(["member"])
})]);

export const zUserEntitiesView = z.strictObject({
  ["members"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zMemberSummary, {}),
  ["network"]: zNetworkSummary,
  ["nodes"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zNodeInventoryView, {})
});

export const zUserEvent = z.union([z.strictObject({
  ["batch"]: z.string(),
  ["configuration"]: zUserConfigurationView,
  ["progress"]: zConfigurationApplyProgress.nullable().exactOptional(),
  ["requestId"]: zRequestId,
  ["type"]: z.enum(["config.snapshot"])
}), z.strictObject({
  ["batch"]: z.string(),
  ["entities"]: zUserEntitiesView,
  ["type"]: z.enum(["entities.snapshot"])
}), z.strictObject({
  ["batch"]: z.string(),
  ["state"]: zStateView,
  ["type"]: z.enum(["state.snapshot"])
}), z.strictObject({
  ["batch"]: z.string(),
  ["cursor"]: z.string(),
  ["disconnectAfterMs"]: z.int().gte(0).lte(2147483647),
  ["heartbeatIntervalMs"]: z.int().gte(0).lte(2147483647),
  ["type"]: z.enum(["sync.ready"])
}), z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable().exactOptional(),
  ["baseCursor"]: z.string(),
  ["configurationRevision"]: zRevision.nullable().exactOptional(),
  ["cursor"]: z.string(),
  ["links"]: z.array(zAggregatedLink),
  ["paths"]: z.array(zAggregatedPath),
  ["removed"]: z.array(zNodeId),
  ["sequence"]: zSequence.nullable().exactOptional(),
  ["stateEpoch"]: zStateEpoch,
  ["type"]: z.enum(["state.delta"]),
  ["updated"]: typedMap(z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u")), zNodeStateView, {})
}), z.strictObject({
  ["baseCursor"]: z.string(),
  ["cursor"]: z.string(),
  ["removed"]: zEntityRemovals,
  ["type"]: z.enum(["entities.delta"]),
  ["updated"]: zUserEntitiesView
}), z.strictObject({
  ["actor"]: zEventActor,
  ["cause"]: zConfigurationChangeCause,
  ["changedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["changedPointers"]: z.array(z.string()),
  ["changesComplete"]: z.boolean(),
  ["networkId"]: zNetworkId,
  ["previousRevision"]: zRevision,
  ["requestId"]: zRequestId.nullable().exactOptional(),
  ["revision"]: zRevision,
  ["sourceChanged"]: z.boolean(),
  ["type"]: z.enum(["configuration.changed"])
}), z.strictObject({
  ["progress"]: zConfigurationApplyProgress,
  ["type"]: z.enum(["configuration.progress"])
}), z.strictObject({
  ["networkId"]: zNetworkId,
  ["revision"]: zRevision,
  ["type"]: z.enum(["resources.changed"])
}), z.strictObject({
  ["observation"]: zObservationTask,
  ["type"]: z.enum(["observation.status"])
}), z.strictObject({
  ["observation"]: zObservationTask,
  ["type"]: z.enum(["observation.sample"])
}), z.strictObject({
  ["subscriptionId"]: zObservationSubscriptionId,
  ["type"]: z.enum(["observation.removed"])
}), z.strictObject({
  ["reason"]: zResetReason,
  ["type"]: z.enum(["sync.reset"])
}), z.strictObject({
  ["reason"]: zClosedReason,
  ["type"]: z.enum(["stream.closed"])
})]);

export const zValidateConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zVlessListener = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["flow"]: z.string().exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zVlessStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zListenerTls,
  ["uuid"]: zSecretString.exactOptional(),
  ["websocket"]: zListenerWebSocket.exactOptional()
});

export const zVlessListenerUpdate = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["endpoints"]: z.array(zEndpoint).exactOptional(),
  ["flow"]: z.string().exactOptional(),
  ["listen"]: z.string().exactOptional(),
  ["listenPort"]: zPort,
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zVlessStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zListenerTls,
  ["uuid"]: zSecretString.exactOptional(),
  ["websocket"]: zListenerWebSocketUpdate.nullable().exactOptional()
});

export const zWatchNetworkPath = z.strictObject({
  ["networkId"]: zNetworkId
});
