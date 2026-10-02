import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { fstatSync } from "node:fs";
import { lstat, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { ControlApiError, listNetworks, listNodes, createEnrollmentToken, getConnections, saveConnections, getManagedListeners, saveManagedListener, revokeNode } from "../../packages/control-client/src/index.ts";
import { getNetworkPolicyOperation, saveNetworkPolicyOperation, getAgentDataplaneOperation } from "../../packages/control-client/src/generated/operations.gen.ts";
import type { NetworkPolicy } from "../../packages/control-client/src/generated/types.gen.ts";
import { command, parse, text, boolean } from "../managed-connections/local.ts";
import { fixtureNetworkId } from "../managed-connections/network-scope.ts";
import { local, probe, delivered, denied, noFallback, tunReady, ROLES } from "../managed-dataplane/checks.ts";
import type { ByRole, Role } from "../managed-dataplane/checks.ts";

const HERE=dirname(fileURLToPath(import.meta.url)); const ROOT=resolve(HERE,"../..");
const TEMP="/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode";
const image=process.argv[2]; assert(image && /^meshway-managedtest:[a-zA-Z0-9_.-]+$/.test(image));
process.umask(0o077);
const fd=Number(process.env.MESHWAY_DATAPLANE_LOCK_FD);assert(Number.isSafeInteger(fd)&&fd>=3);
const lock=await lstat(process.env.MESHWAY_PERF_LOCK!);const held=fstatSync(fd);assert.equal(lock.ino,held.ino);assert.equal(lock.dev,held.dev);
const run=`${Date.now().toString(36)}-${randomBytes(6).toString("hex")}`;const prefix=`meshway-p3a-${run}`;
const artifacts=await mkdtemp(join(TEMP,`${prefix}-`));
const names={NYC:`${prefix}-a`,LAX:`${prefix}-b`,SHA:`${prefix}-c`,controller:`${prefix}-controller`};
const network=`${prefix}-net`;const created:string[]=[];let networkCreated=false;
const ids:ByRole<string>={NYC:"",LAX:"",SHA:""};const ips:ByRole<string>={NYC:"",LAX:"",SHA:""};const bearers:ByRole<string>={NYC:"",LAX:"",SHA:""};
const evidence:object[]=[];let status="FAIL";const cleanupErrors:string[]=[];
const docker=(args:string[],input?:string)=>command("docker",args,{cwd:ROOT,timeout:120_000,...(input===undefined?{}:{input})});
const helper=async(role:Role|"controller",action:string,data:object={})=>parse(await docker(["exec","-i",names[role],"python3","/private/harness.py",action],JSON.stringify(data)));
async function until<T>(name:string,f:()=>Promise<T|false>,timeout=80_000):Promise<T> {
  const end=Date.now()+timeout;do {const value=await f();if(value!==false)return value;await delay(500);}while(Date.now()<end);throw new Error(`deadline: ${name}`);
}
const observe=async(role:Role)=>local(await helper(role,"observe",{destinations:ROLES.filter(peer=>peer!==role).map(peer=>ips[peer]).filter(Boolean)}));
const one=async(source:Role,destination:"NYC"|"SHA")=>probe(await helper(source,"probe",{address:ips[destination],body:`meshway-tun:${run}:${destination}\n`}));
async function capture(name:string,notes:string[]=[]) {evidence.push({name,at:new Date().toISOString(),notes,local:await Promise.all(ROLES.map(observe)),plans:await Promise.all(ROLES.map(role=>helper(role,"execution-proof")))});}
try {
  const imageId=(await docker(["image","inspect",image,"--format","{{.Id}}"])).trim();assert(/^sha256:[a-f0-9]{64}$/.test(imageId));
  evidence.push({name:"immutable-image",image_reference:image,image_id:imageId});
  await docker(["network","create","--label",`io.meshway.p3a=${run}`,network]);networkCreated=true;
  for(const role of ["controller",...ROLES] as const) {
    const controller=role==="controller";
    await docker(["create","--init","--name",names[role],"--hostname",role.toLowerCase(),"--label",`io.meshway.p3a=${run}`,"--network",network,
      ...(controller?["--network-alias","controller.mesh.test","--publish","127.0.0.1::8420"]:role==="LAX"?["--network-alias","lax.mesh.test"]:[]),
      "--cap-drop","ALL",...(!controller?["--cap-add","NET_ADMIN","--device","/dev/net/tun:/dev/net/tun:rw"]:[]),"--security-opt","no-new-privileges","--read-only","--tmpfs","/private:rw,nosuid,nodev,noexec,mode=0700,size=96m",
      "--sysctl","net.ipv4.ip_forward=0","--sysctl","net.ipv6.conf.all.forwarding=0",...(!controller?["--sysctl","net.ipv6.conf.default.disable_ipv6=1"]:[]),"--sysctl","net.ipv4.ping_group_range=0 2147483647","--env","PYTHONDONTWRITEBYTECODE=1",imageId]);
    created.push(names[role]);await docker(["start",names[role]]);
    for(const [name,path] of [["harness.py",join(HERE,"container.py")],["canaries.py",join(HERE,"../managed-dataplane/container.py")]]) {
      assert(name&&path);await docker(["exec","-i",names[role],"python3","-c",`import os,sys;os.umask(0o077);open('/private/${name}','x').write(sys.stdin.read())`],await readFile(path,"utf8"));
    }
  }
  await helper("LAX","controller-proxy-start");await helper("controller","controller-start");
  const baseUrl=`http://${(await docker(["port",names.controller,"8420/tcp"])).trim()}`;
  const admin=await until("bootstrap",async()=>{try{return text((await helper("controller","admin-token")).token);}catch{return false;}});
  const networks=await until("normal controller API",async()=>{try{return await listNetworks(baseUrl,admin);}catch{return false;}});const scope=fixtureNetworkId(networks);
  for(const role of ROLES) {
    const token=await createEnrollmentToken(baseUrl,admin,scope,{expires_in_seconds:300});
    ids[role]=text((await helper(role,"enroll",{name:role,token:token.token})).node_id);
    bearers[role]=text((await helper(role,"node-bearer")).token);await helper(role,"agent-start");
  }
  const inventory=await until("bound public keys",async()=>{const c=await getConnections(baseUrl,admin,scope);return c.nodes.length===3 && c.nodes.every(n=>n.public_key!==null)?c:false;});
  for(const role of ROLES){const node=inventory.nodes.find(n=>n.node.id===ids[role]);assert(node);ips[role]=node.node.mesh_ipv4;}
  const addr:ByRole<string>={NYC:"",LAX:"",SHA:""};for(const role of ROLES) addr[role]=(await docker(["inspect",names[role],"--format",`{{(index .NetworkSettings.Networks "${network}").IPAddress}}`])).trim();
  // Controller-managed UDP listeners on B and C. No local profile/transport configuration.
  for(const role of ["LAX","SHA"] as const) {
    await saveManagedListener(baseUrl,admin,scope,{expected_revision:(await getManagedListeners(baseUrl,admin,scope)).revision,settings:{id:`p3a-${role}`,node_id:ids[role],enabled:true,transport:"native_udp",bind_address:"0.0.0.0",bind_port:41000,advertised_endpoints:[{host:addr[role],port:41000}],vless:null}});
  }
  const currentConnections=await getConnections(baseUrl,admin,scope);
  await saveConnections(baseUrl,admin,scope,{expected_revision:currentConnections.revision,configuration:{listeners:currentConnections.configuration.listeners,candidates:[
    {id:"a-b",dialer_node_id:ids.NYC,listener_id:"p3a-LAX",endpoint:{host:addr.LAX,port:41000},priority:0,credential_ref:null},
    {id:"c-b",dialer_node_id:ids.SHA,listener_id:"p3a-LAX",endpoint:{host:addr.LAX,port:41000},priority:0,credential_ref:null},
    {id:"a-c",dialer_node_id:ids.NYC,listener_id:"p3a-SHA",endpoint:{host:addr.SHA,port:41000},priority:0,credential_ref:null},
  ]}});
  const policy:NetworkPolicy={connections:[{id:"allow",a:"*",b:"*",action:"allow"}],grants:[{id:"endpoints",src:[`node:${ids.NYC}`],dst:[`node:${ids.SHA}`],bidirectional:true}],mesh:[],public_rules:[],profiles:[],assignments:[],automatic:{regions:{},fallback:"mesh-only"}};
  async function put(p:NetworkPolicy){return saveNetworkPolicyOperation(baseUrl,admin,scope,{expected_revision:(await getNetworkPolicyOperation(baseUrl,admin,scope)).revision,policy:p,source_jsonc:null});}
  async function ready() {
    return until("normal run v3 ACK and active TUN",async()=>{
      const revision=(await getNetworkPolicyOperation(baseUrl,admin,scope)).revision;
      const proof=await Promise.all(ROLES.map(role=>helper(role,"execution-proof")));
      const observations=await Promise.all(ROLES.map(observe));
      for(const [i,role] of ROLES.entries()) assert(observations[i]?.agent_pid!==null,`${role} normal Agent run exited`);
      return ROLES.every((role,i)=>proof[i]?.revision===revision && proof[i]?.valid===true && tunReady(observations[i]!,ids[role],ips[role],revision,text(proof[i]!.digest)))?{revision,proof}:false;
    });
  }
  async function success(name:string){await ready();await until(name,async()=>{const a=await one("NYC","SHA");const c=await one("SHA","NYC");return delivered(a)&&delivered(c)?true:false;});await capture(name);}
  async function deny(name:string){const a=await one("NYC","SHA");const c=await one("SHA","NYC");assert(denied(a)&&denied(c));await capture(name);}
  await put(policy);await ready();
  for(const role of ["NYC","SHA"] as const){await helper(role,"http-start",{address:ips[role],body:`meshway-tun:${run}:${role}\n`});await until("local Mesh server",async()=>boolean((await helper(role,"self-http",{address:ips[role],body:`meshway-tun:${run}:${role}\n`})).ready));}
  await success("no Mesh rule selects direct candidate");
  const through={...policy,mesh:[{id:"first-via-b",src:"*",dst:"*",via:[`node:${ids.LAX}`]},{id:"later-direct",src:"*",dst:"*",via:[]}]};
  await put(through);await success("first-match mandatory B despite direct A-C candidate");
  const linted=await getNetworkPolicyOperation(baseUrl,admin,scope);
  assert(linted.diagnostics.some(d=>d.field==="mesh.later-direct"&&d.message.includes("fully")&&d.message.includes("example node:")&&d.message.includes("first-via-b")),"published source diagnostics must include a real inventory-dependent shadow example");
  const selected=await helper("NYC","execution-proof");assert(JSON.stringify(selected.paths).includes(ids.LAX));
  const old=await getAgentDataplaneOperation(baseUrl,bearers.NYC);assert(!old.payload.access_valid&&!old.payload.configuration.policy_valid&&old.payload.access_policy.grants.length===0);
  await put({...policy,mesh:[through.mesh[1]!,through.mesh[0]!]});await success("reordering first-match selects direct");
  assert(!JSON.stringify((await helper("NYC","execution-proof")).paths).includes(ids.LAX));
  await put({...through,grants:[{...policy.grants[0]!,bidirectional:false}]});await ready();
  const before=(await observe("SHA")).icmp.InEchos;
  // TUN installation is not authenticated channel/endpoint-session readiness.
  // Retry actual packets until forward delivery is proven; every attempted
  // round trip and reverse initiation must remain denied throughout recovery.
  await until("actual one-way request reaches kernel",async()=>{
    assert(denied(await one("NYC","SHA")));assert(denied(await one("SHA","NYC")));
    return (await observe("SHA")).icmp.InEchos>before?true:false;
  },40_000);
  await capture("one-way grant delivers request but denies stateful return and reverse initiation");
  await capture("one-way grant boundary",["Stateful Mesh return is not implemented; independent reverse initiation is denied. No reverse grant was synthesized."]);
  await put({...through,grants:[]});await ready();await deny("empty grants deny all");
  const union={...through,grants:[
    {id:"forward",src:[`node:${ids.NYC}`],dst:[`node:${ids.SHA}`],bidirectional:false},
    {id:"reverse",src:[`node:${ids.SHA}`],dst:[`node:${ids.NYC}`],bidirectional:false},
  ]};
  await put(union);await success("independent one-way grants form an additive union");
  await put({...union,grants:[...union.grants].reverse()});await success("grant reorder preserves both authorized directions");
  await put({...through,connections:[...policy.connections,{id:"block-b-c",a:`node:${ids.LAX}`,b:`node:${ids.SHA}`,action:"block"}]});await ready();await deny("connection block dominates allow and cannot bypass mandatory B");
  const blockedEdit=await getConnections(baseUrl,admin,scope);
  assert(blockedEdit.configuration.candidates.some(c=>c.id==="c-b"),"management must retain blocked saved intent");
  await saveConnections(baseUrl,admin,scope,{expected_revision:blockedEdit.revision,configuration:blockedEdit.configuration});
  assert((await getConnections(baseUrl,admin,scope)).configuration.candidates.some(c=>c.id==="c-b"));
  await put(through);await success("restore explicitly authorized mandatory path");
  assert((await getConnections(baseUrl,admin,scope)).configuration.candidates.some(c=>c.id==="c-b"));
  await capture("C1 block GET-save unblock restores original candidate without rewriting it");
  await helper("NYC","agent-stop");await helper("NYC","agent-start");await success("restart renegotiates runtime epoch and never activates saved cache offline");
  await helper("NYC","agent-stop");
  await put({...through,grants:[]}); // Genuine revision advance while NYC is stopped.
  await helper("NYC","agent-start");await ready();await deny("A1 offline revision change then normal restart installs current denied plan");
  await put(through);await success("A1 restarted daemon receives restored grants without clearing its journal");
  await helper("LAX","stall",{mode:"execution"});await put({...through,grants:[]});await delay(4_000);await deny("heartbeat higher revision fences before stalled execution GET");
  await helper("LAX","stall",{mode:"off"});await ready();await put(through);await success("recover from execution HTTP stall");
  await helper("LAX","stall",{mode:"all"});await delay(18_000);await deny("independent watchdog expires while HTTP stalls");
  for(const role of ROLES) assert((await observe(role)).routes.every(noFallback),"Mesh pool remains TUN sink or guard");
  await helper("LAX","stall",{mode:"off"});await success("recover after lease expiry");
  // Normal managed listeners and a running owner's generated download. The
  // 65th mutation must roll back; the owner must still consume the 64 plan.
  for(let i=0;i<64;i++) {
    const revision=(await getManagedListeners(baseUrl,admin,scope)).revision;
    await saveManagedListener(baseUrl,admin,scope,{expected_revision:revision,settings:{id:`budget-${i}`,node_id:ids.NYC,enabled:true,transport:"vless",bind_address:"0.0.0.0",bind_port:21000+i,advertised_endpoints:[],vless:{stream:"tcp",server_name:"meshway.test",websocket_path:null,websocket_host:null}}});
  }
  const budgetBefore=await getManagedListeners(baseUrl,admin,scope);
  await assert.rejects(saveManagedListener(baseUrl,admin,scope,{expected_revision:budgetBefore.revision,settings:{id:"budget-64",node_id:ids.NYC,enabled:true,transport:"vless",bind_address:"0.0.0.0",bind_port:21064,advertised_endpoints:[],vless:{stream:"tcp",server_name:"meshway.test",websocket_path:null,websocket_host:null}}}),e=>e instanceof ControlApiError&&e.status===422);
  const budgetAfter=await getManagedListeners(baseUrl,admin,scope);assert.equal(budgetAfter.revision,budgetBefore.revision);assert(!budgetAfter.listeners.some(l=>l.settings.id==="budget-64"));
  await ready();await until("normal owner consumes exactly 64 managed materials",async()=>(await helper("NYC","execution-proof")).material_count===64);
  await success("C2 rejected 65th listener preserves current 64-material normal TUN execution");
  await revokeNode(baseUrl,admin,scope,{node_id:ids.LAX});await delay(5_000);await deny("withdraw mandatory relay cannot fall back to direct candidate");
  const remaining=await listNodes(baseUrl,admin,scope);assert(remaining.nodes.some(n=>n.id===ids.LAX&&n.revoked_at!==null));
  status="PASS";
} catch(error) {
  evidence.push({failure:error instanceof Error?error.message:"unknown"});
  try {await capture("failure-observations");evidence.push({errors:await Promise.all(ROLES.map(role=>helper(role,"agent-errors")))});}catch{/* Partial setup may have no helper. */}
  throw error;
}
finally {
  for(const name of created.reverse())try{await docker(["rm","-f",name]);}catch{cleanupErrors.push(name);}
  if(networkCreated)try{await docker(["network","rm",network]);}catch{cleanupErrors.push(network);}
  await writeFile(join(artifacts,"result.json"),JSON.stringify({run,status,evidence,cleanupErrors},null,2),{mode:0o600});console.log(`P3a ${status}: ${artifacts}`);
}
