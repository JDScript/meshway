// Only for ephemeral native App fixtures. These sessions never enter the App;
// it receives the same PKCE result as it would from a signed-in browser user.
import { randomBytes } from "node:crypto";
import {
  acceptAccountInvitation,
  createAccountInvitation,
  loginAccount,
  registerInvitedAccount,
} from "../../../packages/control-client/src/index.ts";

export async function createFixtureUserSession(base: string, admin: string, networks: readonly string[]): Promise<string> {
  const first = networks[0];
  if (!first) throw new Error("A native fixture needs a destination network");
  const email = `native-${randomBytes(12).toString("hex")}@example.test`;
  const password = randomBytes(32).toString("hex");
  const invitation = await createAccountInvitation(base, admin, first, { email, role: "member", expires_in_seconds: 3600 });
  await registerInvitedAccount(base, { email, password, invitation_token: invitation.invitation_token });
  const session = await loginAccount(base, { email, password });
  for (const network of networks.slice(1)) {
    const invitation = await createAccountInvitation(base, admin, network, { email, role: "member", expires_in_seconds: 3600 });
    await acceptAccountInvitation(base, session.session_token, { invitation_token: invitation.invitation_token });
  }
  return session.session_token;
}
