// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import * as z from "zod";

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

export const zAccessDefault = z.enum(["deny"]);

export const zAccessRule = z.strictObject({
  ["bidirectional"]: z.boolean().exactOptional(),
  ["dst"]: z.array(z.string()),
  ["ip"]: z.array(z.string()),
  ["remark"]: z.string().exactOptional(),
  ["src"]: z.array(z.string())
});

export const zRequestId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

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

export const zActivityAction = z.enum(["account.register","session.login","session.logout","session.revoke","session.revokeOthers","session.revokeAll","account.password","account.passwordReset","account.resetIssued","account.email","network.create","network.rename","configuration.apply","membership.role","membership.join","membership.remove","joinLink.create","joinLink.revoke"]);

export const zActivityActor = z.union([z.strictObject({
  ["type"]: z.enum(["user"]),
  ["userId"]: zUserId
}), z.strictObject({
  ["type"]: z.enum(["system"])
})]);

export const zRevision = z.int().gte(1).lte(9007199254740991);

export const zActivityId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zNetworkId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zMembershipId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zJoinLinkId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zActivityTarget = z.union([z.strictObject({
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

export const zAddressPools = z.strictObject({
  ["ipv4"]: z.array(z.string()),
  ["ipv6"]: z.array(z.string())
});

export const zAddressPoolsInput = z.strictObject({
  ["ipv4"]: z.array(z.string()).exactOptional(),
  ["ipv6"]: z.array(z.string()).exactOptional()
});

export const zApplyConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
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

export const zVersion = z.int().gte(1).lte(9007199254740991);

export const zMembershipRole = z.enum(["owner","admin","member"]);

export const zChangeMemberRoleRequest = z.strictObject({
  ["expectedVersion"]: zVersion,
  ["role"]: zMembershipRole
});

export const zChangePasswordRequest = z.strictObject({
  ["currentPassword"]: zWireSecret,
  ["password"]: zWireSecret
});

export const zCompareConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zCompareConfigurationRequest = z.strictObject({
  ["fromRevision"]: zRevision,
  ["toRevision"]: zRevision
});

export const zNodeId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zPublicationId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zConfigurationAppliedResponse = z.strictObject({
  ["deletedNodeIds"]: z.array(zNodeId),
  ["networkId"]: zNetworkId,
  ["publicationIds"]: z.array(zPublicationId),
  ["requestId"]: zRequestId,
  ["revision"]: zRevision,
  ["targetCount"]: z.int().gte(0).lte(2147483647)
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

export const zConfigurationIssue = z.strictObject({
  ["message"]: z.string(),
  ["path"]: z.string()
});

export const zTopologyAction = z.enum(["allow","block"]);

export const zTopologyRule = z.strictObject({
  ["a"]: z.string(),
  ["action"]: zTopologyAction,
  ["b"]: z.string(),
  ["remark"]: z.string().exactOptional()
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

export const zRouteMatch = z.strictObject({
  ["domainSuffix"]: z.array(z.string()).exactOptional(),
  ["ipCIDR"]: z.array(z.string()).exactOptional(),
  ["ruleSets"]: z.array(z.string()).exactOptional()
});

export const zEgressRule = z.strictObject({
  ["action"]: zRouteAction,
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

export const zPort = z.int().gte(1).lte(65535);

export const zDnsTransport = z.enum(["udp","tcp"]);

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

export const zPositiveDuration = z.string().regex(new RegExp("^(?:(?:[1-9][0-9]{0,18}|1[0-7][0-9]{18}|18[0-3][0-9]{17}|184[0-3][0-9]{16}|1844[0-5][0-9]{15}|18446[0-6][0-9]{14}|184467[0-3][0-9]{13}|1844674[0-3][0-9]{12}|184467440[0-6][0-9]{10}|1844674407[0-2][0-9]{9}|18446744073[0-6][0-9]{8}|1844674407370[0-8][0-9]{6}|18446744073709[0-4][0-9]{5}|184467440737095[0-4][0-9]{4}|1844674407370955[0-0][0-9]{3}|18446744073709551[0-5][0-9]{2}|184467440737095516[0-0][0-9]{1}|1844674407370955161[0-4][0-9]{0}|18446744073709551615)ms|(?:[1-9][0-9]{0,15}|1[0-7][0-9]{15}|18[0-3][0-9]{14}|184[0-3][0-9]{13}|1844[0-5][0-9]{12}|18446[0-6][0-9]{11}|184467[0-3][0-9]{10}|1844674[0-3][0-9]{9}|184467440[0-6][0-9]{7}|1844674407[0-2][0-9]{6}|18446744073[0-6][0-9]{5}|1844674407370[0-8][0-9]{3}|18446744073709[0-4][0-9]{2}|184467440737095[0-4][0-9]{1}|1844674407370955[0-0][0-9]{0}|18446744073709551)s|(?:[1-9][0-9]{0,13}|[1-2][0-9]{14}|30[0-6][0-9]{12}|307[0-3][0-9]{11}|3074[0-3][0-9]{10}|30744[0-4][0-9]{9}|307445[0-6][0-9]{8}|3074457[0-2][0-9]{7}|30744573[0-3][0-9]{6}|307445734[0-4][0-9]{5}|3074457345[0-5][0-9]{4}|30744573456[0-0][0-9]{3}|307445734561[0-7][0-9]{2}|3074457345618[0-1][0-9]{1}|30744573456182[0-4][0-9]{0}|307445734561825)m|(?:[1-9][0-9]{0,11}|[1-4][0-9]{12}|5[0-0][0-9]{11}|51[0-1][0-9]{10}|512[0-3][0-9]{9}|51240[0-8][0-9]{7}|512409[0-4][0-9]{6}|5124095[0-4][0-9]{5}|51240955[0-6][0-9]{4}|512409557[0-5][0-9]{3}|51240955760[0-2][0-9]{1}|5124095576030)h)(?![\\s\\S])", "u"));

export const zSocketOptions = z.strictObject({
  ["receiveBuffer"]: z.int().gte(0).lte(2147483647).exactOptional(),
  ["sendBuffer"]: z.int().gte(0).lte(2147483647).exactOptional()
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
  ["priority"]: z.int().gte(0).lte(2147483647).exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional()
});

export const zNativeUdpDialer = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["priority"]: z.int().gte(0).lte(2147483647).exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional()
});

export const zTlsVersion = z.enum(["1.2","1.3"]);

export const zTlsTrust = z.enum(["managed"]);

export const zDialerTls = z.strictObject({
  ["alpn"]: z.array(z.string()).exactOptional(),
  ["handshakeTimeout"]: zPositiveDuration.exactOptional(),
  ["maxVersion"]: zTlsVersion.exactOptional(),
  ["minVersion"]: zTlsVersion.exactOptional(),
  ["trust"]: zTlsTrust.exactOptional()
});

export const zTrojanDialer = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["priority"]: z.int().gte(0).lte(2147483647).exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zDialerTls.exactOptional()
});

export const zDialerStream = z.enum(["auto","tcp","websocket"]);

export const zWebSocketHeaders = typedMap(z.string().regex(new RegExp("^(?!(?:[Hh][Oo][Ss][Tt]|[Cc][Oo][Nn][Nn][Ee][Cc][Tt][Ii][Oo][Nn]|[Uu][Pp][Gg][Rr][Aa][Dd][Ee]|[Cc][Oo][Nn][Tt][Ee][Nn][Tt]-[Ll][Ee][Nn][Gg][Tt][Hh]|[Tt][Rr][Aa][Nn][Ss][Ff][Ee][Rr]-[Ee][Nn][Cc][Oo][Dd][Ii][Nn][Gg])(?![\\s\\S]))(?![Ss][Ee][Cc]-[Ww][Ee][Bb][Ss][Oo][Cc][Kk][Ee][Tt]-)[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])", "u")), z.string().regex(new RegExp("^[^\\u0000-\\u0008\\u000A-\\u001F\\u007F-\\u009F]*(?![\\s\\S])", "u")), {});

export const zDialerWebSocketUpdate = z.strictObject({
  ["headers"]: zWebSocketHeaders.exactOptional()
});

export const zVlessDialerUpdate = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["priority"]: z.int().gte(0).lte(2147483647).exactOptional(),
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

export const zTopologyDefault = z.enum(["allow","deny"]);

export const zDefaultsUpdate = z.strictObject({
  ["access"]: zAccessDefault.exactOptional(),
  ["dialers"]: zDialerSetUpdate,
  ["topology"]: zTopologyDefault.exactOptional()
});

export const zRuleSetFormat = z.enum(["sing-box-source","mihomo-domain","mihomo-ipcidr"]);

export const zRuleSet = z.union([z.strictObject({
  ["domainSuffix"]: z.array(z.string()).exactOptional(),
  ["ipCIDR"]: z.array(z.string()).exactOptional(),
  ["type"]: z.enum(["inline"])
}), z.strictObject({
  ["format"]: zRuleSetFormat,
  ["type"]: z.enum(["remote"]),
  ["updateInterval"]: zPositiveDuration.exactOptional(),
  ["url"]: z.string()
})]);

export const zNodeSettingsPatch = z.strictObject({
  ["dialers"]: zDialerSetUpdate.nullable().exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["exitEnabled"]: z.boolean().exactOptional(),
  ["hostname"]: z.string().exactOptional(),
  ["remark"]: z.string().exactOptional()
});

export const zEndpoint = z.strictObject({
  ["host"]: z.string(),
  ["port"]: zPort
});

export const zVlessStream = z.enum(["tcp","websocket"]);

export const zSecretString = z.string();

export const zTlsMode = z.enum(["managed","provided"]);

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

export const zWebSocketPath = z.string().regex(new RegExp("^/[^\\u0000-\\u0020\\u007F-\\u009F\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000?#]*(?![\\s\\S])", "u"));

export const zListenerWebSocket = z.strictObject({
  ["host"]: z.string().exactOptional(),
  ["path"]: zWebSocketPath.exactOptional(),
  ["responseHeaders"]: zWebSocketHeaders.exactOptional()
});

export const zTrojanStream = z.enum(["tcp"]);

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

export const zListenerUpdate = zListener;

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

export const zVlessDialerView = z.strictObject({
  ["bindInterface"]: z.string(),
  ["connectTimeout"]: zPositiveDuration,
  ["enabled"]: z.boolean(),
  ["localAddress"]: z.string(),
  ["priority"]: z.int().gte(0).lte(2147483647),
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

export const zDefaultsView = z.strictObject({
  ["access"]: zAccessDefault,
  ["dialers"]: zDialerSetView,
  ["topology"]: zTopologyDefault
});

export const zNodeAddresses = z.strictObject({
  ["ipv4"]: z.ipv4().nullable(),
  ["ipv6"]: z.ipv6().nullable()
});

export const zNodeIdentity = z.union([z.strictObject({
  ["type"]: z.enum(["user"]),
  ["user"]: z.string()
}), z.strictObject({
  ["tags"]: z.array(z.string()),
  ["type"]: z.enum(["tag"])
})]);

export const zTransportProtocol = z.enum(["native_udp","native_tcp","vless","trojan"]);

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

export const zConfigurationPreviewResponse = z.strictObject({
  ["addressChangedNodeIds"]: z.array(zNodeId),
  ["baseRevision"]: zRevision,
  ["changeCount"]: z.int().gte(0).lte(2147483647),
  ["changes"]: z.array(zConfigurationChange),
  ["deletedNodeIds"]: z.array(zNodeId),
  ["preview"]: zConfigurationView,
  ["requestId"]: zRequestId,
  ["sourceChanged"]: z.boolean(),
  ["unavailableDependencies"]: z.array(zConfigurationIssue)
});

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

export const zDialerWebSocket = z.strictObject({
  ["headers"]: zWebSocketHeaders.exactOptional()
});

export const zVlessDialer = z.strictObject({
  ["bindInterface"]: z.string().exactOptional(),
  ["connectTimeout"]: zPositiveDuration.exactOptional(),
  ["enabled"]: z.boolean().exactOptional(),
  ["localAddress"]: z.string().exactOptional(),
  ["priority"]: z.int().gte(0).lte(2147483647).exactOptional(),
  ["remark"]: z.string().exactOptional(),
  ["socket"]: zSocketOptions.exactOptional(),
  ["stream"]: zDialerStream.exactOptional(),
  ["tcp"]: zTcpOptions.exactOptional(),
  ["tls"]: zDialerTls.exactOptional(),
  ["websocket"]: zDialerWebSocket.exactOptional()
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

export const zMaterialRequirement = z.enum(["uuid","password","certificate","privateKey","certificateValidation"]);

export const zPreparationKind = z.union([z.enum(["addressPoolV4","addressPoolV6","addressV4","addressV6","identityProof","ruleSetContent"]), z.strictObject({
  ["material"]: zMaterialRequirement
})]);

export const zPreparationRequirement = z.strictObject({
  ["kind"]: zPreparationKind,
  ["path"]: z.string()
});

export const zConfigurationValidationParams = z.strictObject({
  ["issues"]: z.array(zConfigurationIssue),
  ["requirements"]: z.array(zPreparationRequirement)
});

export const zCount = z.int().gte(0).lte(9007199254740991);

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

export const zEmptyParams = z.strictObject({

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

export const zGetConfigurationPath = z.strictObject({
  ["networkId"]: zNetworkId
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

export const zGetLeaveImpactPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zGetMemberRemovalImpactPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["membershipId"]: zMembershipId
});

export const zGetNetworkActivityPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["activityId"]: zActivityId
});

export const zGetNetworkPath = z.strictObject({
  ["networkId"]: zNetworkId
});

export const zJoinLinkView = z.strictObject({
  ["consumedAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time").nullable(),
  ["consumedByUserId"]: zUserId.nullable(),
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["id"]: zJoinLinkId,
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
  ["link"]: zJoinLinkView,
  ["networkName"]: z.string(),
  ["requestId"]: zRequestId
});

export const zJoinLinkListResponse = z.strictObject({
  ["asOf"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["items"]: z.array(zJoinLinkView),
  ["nextCursor"]: z.string().nullable(),
  ["requestId"]: zRequestId
});

export const zJoinLinkRedeemed = z.strictObject({
  ["membershipId"]: zMembershipId,
  ["networkId"]: zNetworkId,
  ["requestId"]: zRequestId
});

export const zLeaveNetworkPath = z.strictObject({
  ["networkId"]: zNetworkId
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

export const zNetworkActions = z.strictObject({
  ["editConfiguration"]: z.boolean(),
  ["manageMembers"]: z.boolean(),
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

export const zRevokeAccountSessionPath = z.strictObject({
  ["sessionId"]: zSessionId
});

export const zRevokeJoinLinkPath = z.strictObject({
  ["networkId"]: zNetworkId,
  ["linkId"]: zJoinLinkId
});

export const zSessionIssuedResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["session"]: zSessionMetadata,
  ["token"]: zWireSecret,
  ["user"]: zUserView
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
