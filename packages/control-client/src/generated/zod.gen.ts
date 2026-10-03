// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.

import * as z from "zod";

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

export const zPasswordLengthUnit = z.enum(["unicodeCodepoints"]);

export const zPasswordRules = z.strictObject({
  ["lengthUnit"]: zPasswordLengthUnit,
  ["maxCodepoints"]: z.int().gte(0).lte(65535),
  ["maxUtf8Bytes"]: z.int().gte(0).lte(65535),
  ["minCodepoints"]: z.int().gte(0).lte(65535)
});

export const zRequestId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zAuthOptionsResponse = z.strictObject({
  ["passwordRules"]: zPasswordRules,
  ["registrationEnabled"]: z.boolean(),
  ["requestId"]: zRequestId
});

export const zSessionId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zSessionMetadata = z.strictObject({
  ["createdAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["expiresAt"]: z.string().refine(rfc3339, "Expected RFC 3339 date-time"),
  ["id"]: zSessionId
});

export const zUserId = z.string().refine(value => [...value].length >= 36, "minLength 36 Unicode code points").refine(value => [...value].length <= 36, "maxLength 36 Unicode code points").regex(new RegExp("^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", "u"));

export const zUserView = z.strictObject({
  ["email"]: z.string(),
  ["id"]: zUserId
});

export const zAuthSessionResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["session"]: zSessionMetadata,
  ["user"]: zUserView
});

export const zEmptyParams = z.strictObject({

});

export const zProblem = z.union([z.strictObject({
  ["code"]: z.enum(["invalid_credentials"]),
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
})]);

export const zErrorResponse = z.strictObject({
  ["problem"]: zProblem,
  ["requestId"]: zRequestId.nullable(),
  ["status"]: z.int().gte(0).lte(65535)
});

export const zWireSecret = z.string();

export const zLoginRequest = z.strictObject({
  ["email"]: z.string(),
  ["password"]: zWireSecret
});

export const zRegisterRequest = z.strictObject({
  ["email"]: z.string(),
  ["password"]: zWireSecret
});

export const zSessionIssuedResponse = z.strictObject({
  ["requestId"]: zRequestId,
  ["session"]: zSessionMetadata,
  ["token"]: zWireSecret,
  ["user"]: zUserView
});
