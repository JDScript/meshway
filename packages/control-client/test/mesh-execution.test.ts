import assert from "node:assert/strict";
import test from "node:test";
import { zMeshExecutionResponse, zMeshExecutionRequest, zSaveNetworkPolicyRequest } from "../src/generated/zod.gen.ts";

test("v3 management source validates the real NetworkPolicy schema and closed variants", () => {
  const request={expected_revision:1,policy:{connections:[{id:"default",a:"*",b:"*",action:"allow"}],grants:[{id:"members",src:["*"],dst:["*"],bidirectional:false}],mesh:[],public_rules:[],profiles:[],assignments:[],automatic:{regions:{},fallback:"mesh-only"}},source_jsonc:null};
  assert(zSaveNetworkPolicyRequest.safeParse(request).success);
  assert(!zSaveNetworkPolicyRequest.safeParse({...request,policy:{...request.policy,compiled_paths:[]}}).success);
  assert(!zSaveNetworkPolicyRequest.safeParse({...request,policy:{...request.policy,connections:[{...request.policy.connections[0],action:"reject"}]}}).success);
});

test("epoch negotiation and inventory-only legacy response remain explicit and closed", () => {
  const request={epoch:1,instance_id:"a".repeat(64),capabilities:["ipv4_exact_path_v1"]};
  assert(zMeshExecutionRequest.safeParse(request).success);
  assert(!zMeshExecutionRequest.safeParse({...request,capabilities:["guess-agent-version"]}).success);
  assert(!zMeshExecutionRequest.safeParse({...request,epoch:0}).success);
  assert(!zMeshExecutionRequest.safeParse({...request,capabilities:["ipv4_exact_path_v1","ipv4_exact_path_v1"]}).success);
  const response={source_mode:"legacy_v2",revision:1,snapshot:null};
  assert.deepEqual(zMeshExecutionResponse.parse(response),response);
  assert(!zMeshExecutionResponse.safeParse({source_mode:"legacy_v2",revision:1}).success);
  assert(!zMeshExecutionResponse.safeParse({...response,execute_legacy:true}).success);
});
