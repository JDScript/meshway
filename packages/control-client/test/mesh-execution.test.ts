import assert from "node:assert/strict";
import test from "node:test";
import { zMeshExecutionResponse, zMeshExecutionRequest, zSaveNetworkPolicyRequest, zNodeConfiguration } from "../src/generated/zod.gen.ts";

test("management source validates the current NetworkPolicy schema and closed variants", () => {
  const request={expected_revision:1,policy:{connections:[{id:"default",a:"*",b:"*",action:"allow"}],grants:[{id:"members",src:["*"],dst:["*"],bidirectional:false}],mesh:[],public_rules:[],profiles:[],assignments:[],automatic:{regions:{},fallback:"mesh-only"}},source_jsonc:null};
  assert(zSaveNetworkPolicyRequest.safeParse(request).success);
  assert(!zSaveNetworkPolicyRequest.safeParse({...request,policy:{...request.policy,compiled_paths:[]}}).success);
  assert(!zSaveNetworkPolicyRequest.safeParse({...request,policy:{...request.policy,connections:[{...request.policy.connections[0],action:"reject"}]}}).success);
});

test("inventory schema accepts only current metadata and never a routing source", () => {
  const metadata={schema_version:2,revision:1,network:{ipv4_prefix:"100.96.0.0/16",ipv6_prefix:"fd00::/64"},node:{id:"a".repeat(64),name:"node",country:null,mesh_ipv4:"100.96.0.1",mesh_ipv6:"fd00::1"},peers:[]};
  assert(zNodeConfiguration.safeParse(metadata).success);
  assert(!zNodeConfiguration.safeParse({...metadata,schema_version:1}).success);
  assert(!zNodeConfiguration.safeParse({...metadata,routing_policy:{default_action:"direct",rules:[]}}).success);
});

test("epoch negotiation requires the current capability and a concrete execution snapshot", () => {
  const request={epoch:1,instance_id:"a".repeat(64),capabilities:["ipv4_stateful_exact_path_v2"]};
  assert(zMeshExecutionRequest.safeParse(request).success);
  assert(!zMeshExecutionRequest.safeParse({...request,capabilities:["guess-agent-version"]}).success);
  assert(!zMeshExecutionRequest.safeParse({...request,epoch:0}).success);
  assert(!zMeshExecutionRequest.safeParse({...request,capabilities:["ipv4_stateful_exact_path_v2","ipv4_stateful_exact_path_v2"]}).success);
  assert(!zMeshExecutionRequest.safeParse({...request,capabilities:["ipv4_exact_path_v1"]}).success);
  const response={source_mode:"legacy_v2",revision:1,snapshot:null};
  assert(!zMeshExecutionResponse.safeParse(response).success);
  assert(!zMeshExecutionResponse.safeParse({source_mode:"legacy_v2",revision:1}).success);
  assert(!zMeshExecutionResponse.safeParse({...response,execute_legacy:true}).success);
});
