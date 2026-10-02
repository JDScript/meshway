import assert from "node:assert/strict";
import test from "node:test";
import {
  zAccountInvitation,
  zLoginRequest,
  zLoginResponse,
  zRedeemPasswordResetRequest,
  zSaveMembershipRequest,
} from "../src/generated/zod.gen.ts";

test("generated account credentials remain distinct and UTF-8 byte policy is not a code-point minimum", () => {
  const password = "密码秘密密码";
  assert.ok(Buffer.byteLength(password) >= 12);
  assert.ok([...password].length < 12);
  assert.deepEqual(zLoginRequest.parse({ email: "user@fixture.example", password }), {
    email: "user@fixture.example", password,
  });
  const random = "a".repeat(64);
  assert.ok(zLoginResponse.safeParse({ session_token: `mws_${random}`, expires_at: 1700000000 }).success);
  for (const session_token of [random, `mwi_${random}`, `mwr_${random}`]) {
    assert.equal(zLoginResponse.safeParse({ session_token, expires_at: 1700000000 }).success, false);
  }
  assert.equal(zRedeemPasswordResetRequest.safeParse({ reset_token: `mws_${random}`, new_password: password }).success, false);
});

test("generated account management requires explicit removals and refreshed metadata has no secret", () => {
  const id = "a".repeat(64);
  assert.equal(zSaveMembershipRequest.safeParse({ user_id: id }).success, false);
  assert.equal(zSaveMembershipRequest.safeParse({ user_id: id, role: null }).success, false);
  assert.ok(zSaveMembershipRequest.safeParse({ user_id: id, role: "remove" }).success);
  const metadata = {
    invitation_id: id, network_id: "a".repeat(32), email: "user@fixture.example", role: "member",
    issuer_user_id: null, expires_at: 1700000000, revoked_at: null, consumed_by: null, consumed_at: null,
  };
  assert.ok(zAccountInvitation.safeParse(metadata).success);
  assert.equal(zAccountInvitation.safeParse({ ...metadata, invitation_token: `mwi_${id}` }).success, false);
  const { revoked_at: _revoked, ...missing } = metadata;
  assert.equal(zAccountInvitation.safeParse(missing).success, false);
});
