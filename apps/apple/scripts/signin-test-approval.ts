// Test-only private IPC helper. stdout is captured by the Swift test, never
// inherited by a terminal/log. The App itself never possesses admin authority.
import { readFile } from "node:fs/promises";
import { authorizeDevice, revokeNode } from "../../../packages/control-client/src/index.ts";

const [mode, controller, adminFile, value] = process.argv.slice(2);
if (!controller || !adminFile || !value) throw new Error("Missing isolated test inputs");
const admin = (await readFile(adminFile, "utf8")).trim();
if (mode === "authorize") {
  const url = new URL(value);
  const required = (name: string): string => {
    const value = url.searchParams.get(name);
    if (!value) throw new Error("Missing authorization parameter");
    return value;
  };
  const response = await authorizeDevice(controller, admin, {
    redirect_uri: required("redirect_uri"), code_challenge: required("code_challenge"),
    name: required("name"), agent_version: required("agent_version"),
  });
  const callback = new URL(required("redirect_uri"));
  callback.searchParams.set("state", required("state"));
  callback.searchParams.set("code", response.code);
  process.stdout.write(callback.toString());
} else if (mode === "revoke") {
  await revokeNode(controller, admin, { node_id: value });
} else { throw new Error("Unknown test operation"); }
