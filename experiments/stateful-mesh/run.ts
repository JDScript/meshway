import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { fstatSync } from "node:fs";
import { lstat, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { listNetworks, createEnrollmentToken, getConnections, saveConnections, getManagedListeners, saveManagedListener, revokeNode } from "../../packages/control-client/src/index.ts";
import { getNetworkPolicyOperation, saveNetworkPolicyOperation } from "../../packages/control-client/src/generated/operations.gen.ts";
import type { NetworkPolicy } from "../../packages/control-client/src/generated/types.gen.ts";
import { command, parse, text, boolean } from "../managed-connections/local.ts";
import { fixtureNetworkId } from "../managed-connections/network-scope.ts";
import { local, probe, delivered, denied, noFallback, tunReady, ROLES } from "../managed-dataplane/checks.ts";
import type { ByRole, Role } from "../managed-dataplane/checks.ts";

const HERE=dirname(fileURLToPath(import.meta.url)), ROOT=resolve(HERE,"../..");
const image=process.argv[2]; assert(image && /^meshway-managedtest:[a-zA-Z0-9_.-]+$/.test(image));
process.umask(0o077);
const fd=Number(process.env.MESHWAY_DATAPLANE_LOCK_FD); assert(Number.isSafeInteger(fd)&&fd>=3);
const held=fstatSync(fd), lock=await lstat(process.env.MESHWAY_PERF_LOCK!); assert.equal(held.ino,lock.ino); assert.equal(held.dev,lock.dev);
const run=`${Date.now().toString(36)}-${randomBytes(6).toString("hex")}`, prefix=`meshway-stateful-${run}`;
const artifacts=await mkdtemp(join("/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode",`${prefix}-`));
const names={NYC:`${prefix}-a`,LAX:`${prefix}-via`,SHA:`${prefix}-b`,controller:`${prefix}-controller`};
const network=`${prefix}-net`, created:string[]=[], evidence:object[]=[], cleanupErrors:string[]=[];
let networkCreated=false, status="FAIL", passed=0;
const ids:ByRole<string>={NYC:"",LAX:"",SHA:""}, ips:ByRole<string>={NYC:"",LAX:"",SHA:""};
const docker=(args:string[],input?:string)=>command("docker",args,{cwd:ROOT,timeout:120_000,...(input===undefined?{}:{input})});
const helper=async(role:Role|"controller",action:string,data:object={})=>{
  try{return parse(await docker(["exec","-i",names[role],"python3","/private/harness.py",action],JSON.stringify(data)));}
  catch{
    try{evidence.push({helper:role,action,error:parse(await docker(["exec",names[role],"cat","/private/fixture-error.json"]))});}catch{}
    throw new Error(`helper ${role} ${action} failed`);
  }
};
async function until<T>(name:string,f:()=>Promise<T|false>,timeout=80_000):Promise<T> {
  const end=Date.now()+timeout; do {const result=await f();if(result!==false)return result;await delay(300);}while(Date.now()<end);throw new Error(`deadline: ${name}`);
}
const observe=async(role:Role)=>local(await helper(role,"observe",{destinations:ROLES.filter(p=>p!==role).map(p=>ips[p]).filter(Boolean)}));
const one=async(source:Role,destination:"NYC"|"SHA")=>probe(await helper(source,"probe",{address:ips[destination],body:`meshway-tun:${run}:${destination}\n`}));
const udp=async(source:Role,destination:"NYC"|"SHA")=>boolean((await helper(source,"udp-probe",{address:ips[destination],body:`udp:${run}:${source}`})).delivered);
async function capture(name:string) { evidence.push({name,at:new Date().toISOString(),local:await Promise.all(ROLES.map(observe)),plans:await Promise.all(ROLES.map(r=>helper(r,"execution-proof")))}); }
try {
  const imageId=(await docker(["image","inspect",image,"--format","{{.Id}}"])).trim();
  assert(process.argv[3], "source build receipt required");
  const receipt=parse(await readFile(process.argv[3],"utf8"));
  assert.equal(receipt.status,"PASS");assert.equal(receipt.image_id,imageId);
  const fixtureSources:Record<string,string>={};
  for(const path of ["run.ts","container.py","../p3a-mesh/container.py","../managed-dataplane/container.py","../managed-connections/container.py"]){fixtureSources[path]=createHash("sha256").update(await readFile(join(HERE,path))).digest("hex");}
  evidence.push({name:"source-image",image,imageId,source_build_receipt:process.argv[3]??null});
  evidence.push({name:"fixture-sources",sha256:fixtureSources});
  await docker(["network","create","--label",`io.meshway.stateful=${run}`,network]); networkCreated=true;
  for(const role of ["controller",...ROLES] as const) {
    const controller=role==="controller";
    await docker(["create","--init","--name",names[role],"--hostname",role.toLowerCase(),"--label",`io.meshway.stateful=${run}`,"--network",network,
      ...(controller?["--network-alias","controller.mesh.test","--publish","127.0.0.1::8420"]:role==="LAX"?["--network-alias","lax.mesh.test"]:[]),
      "--cap-drop","ALL",...(!controller?["--cap-add","NET_ADMIN","--device","/dev/net/tun:/dev/net/tun:rw"]:[]),"--security-opt","no-new-privileges","--read-only","--tmpfs","/private:rw,nosuid,nodev,noexec,mode=0700,size=96m",
      "--cpus","1","--memory","256m","--pids-limit","64","--sysctl","net.ipv4.ip_forward=0","--sysctl","net.ipv6.conf.all.forwarding=0",...(!controller?["--sysctl","net.ipv6.conf.default.disable_ipv6=1"]:[]),"--sysctl","net.ipv4.ping_group_range=0 2147483647","--env","PYTHONDONTWRITEBYTECODE=1",imageId]);
    created.push(names[role]); await docker(["start",names[role]]);
    for(const [name,path] of [["harness.py",join(HERE,"container.py")],["current.py",join(HERE,"../p3a-mesh/container.py")],["canaries.py",join(HERE,"../managed-dataplane/container.py")]]) {
      assert(name&&path);await docker(["exec","-i",names[role],"python3","-c",`import os,sys;os.umask(0o077);open('/private/${name}','x').write(sys.stdin.read())`],await readFile(path,"utf8"));
    }
  }
  await helper("LAX","controller-proxy-start");await helper("controller","controller-start");
  const baseUrl=`http://${(await docker(["port",names.controller,"8420/tcp"])).trim()}`;
  const admin=await until("bootstrap",async()=>{try{return text((await helper("controller","admin-token")).token);}catch{return false;}});
  const scope=fixtureNetworkId(await until("normal controller",async()=>{try{return await listNetworks(baseUrl,admin);}catch{return false;}}));
  for(const role of ROLES) {
    const token=await createEnrollmentToken(baseUrl,admin,scope,{expires_in_seconds:300});
    ids[role]=text((await helper(role,"enroll",{name:role,token:token.token})).node_id);await helper(role,"agent-start");
  }
  const inventory=await until("keys",async()=>{const c=await getConnections(baseUrl,admin,scope);return c.nodes.length===3&&c.nodes.every(n=>n.public_key!==null)?c:false;});
  for(const role of ROLES){const node=inventory.nodes.find(n=>n.node.id===ids[role]);assert(node);ips[role]=node.node.mesh_ipv4;}
  const addr:ByRole<string>={NYC:"",LAX:"",SHA:""};for(const role of ROLES)addr[role]=(await docker(["inspect",names[role],"--format",`{{(index .NetworkSettings.Networks "${network}").IPAddress}}`])).trim();
  for(const role of ["LAX","SHA"] as const)await saveManagedListener(baseUrl,admin,scope,{expected_revision:(await getManagedListeners(baseUrl,admin,scope)).revision,settings:{id:`stateful-${role}`,node_id:ids[role],enabled:true,transport:"native_udp",bind_address:"0.0.0.0",bind_port:41000,advertised_endpoints:[{host:addr[role],port:41000}],vless:null}});
  const connections=await getConnections(baseUrl,admin,scope);
  await saveConnections(baseUrl,admin,scope,{expected_revision:connections.revision,configuration:{listeners:connections.configuration.listeners,candidates:[
    {id:"a-via",dialer_node_id:ids.NYC,listener_id:"stateful-LAX",endpoint:{host:addr.LAX,port:41000},priority:0,credential_ref:null},
    {id:"b-via",dialer_node_id:ids.SHA,listener_id:"stateful-LAX",endpoint:{host:addr.LAX,port:41000},priority:0,credential_ref:null},
    {id:"a-b",dialer_node_id:ids.NYC,listener_id:"stateful-SHA",endpoint:{host:addr.SHA,port:41000},priority:0,credential_ref:null},
  ]}});
  const policy:NetworkPolicy={connections:[{id:"allow",a:"*",b:"*",action:"allow"}],grants:[{id:"one-way",src:[`node:${ids.NYC}`],dst:[`node:${ids.SHA}`],bidirectional:false}],mesh:[],public_rules:[],profiles:[],assignments:[],automatic:{regions:{},fallback:"mesh-only"}};
  async function put(p:NetworkPolicy){return saveNetworkPolicyOperation(baseUrl,admin,scope,{expected_revision:(await getNetworkPolicyOperation(baseUrl,admin,scope)).revision,policy:p,source_jsonc:null});}
  async function ready(){await until("current execution and active TUN",async()=>{
    const revision=(await getNetworkPolicyOperation(baseUrl,admin,scope)).revision;
    const proof=await Promise.all(ROLES.map(role=>helper(role,"execution-proof"))), observations=await Promise.all(ROLES.map(observe));
    return ROLES.every((role,i)=>proof[i]?.revision===revision&&proof[i]?.valid===true&&tunReady(observations[i]!,ids[role],ips[role],revision,text(proof[i]!.digest)))?true:false;
  });}
  async function success(name:string){
    await ready();let attempts=0;
    await until(name,async()=>{
      const forward=await one("NYC","SHA"), datagram=await udp("NYC","SHA");
      attempts++;evidence.push({name,attempt:attempts,forward,udp:datagram});
      return delivered(forward)&&datagram?true:false;
    });
    const reverse=await one("SHA","NYC"), reverseUdp=await udp("SHA","NYC");
    assert(denied(reverse));assert(!reverseUdp);evidence.push({name,reverse,reverseUdp});await capture(name);passed++;
  }
  async function deny(name:string){assert(denied(await one("NYC","SHA")));assert(!await udp("NYC","SHA"));assert(denied(await one("SHA","NYC")));await capture(name);passed++;}
  await put(policy);await ready();
  for(const role of ["NYC","SHA"] as const){await helper(role,"http-start",{address:ips[role],body:`meshway-tun:${run}:${role}\n`});await helper(role,"udp-start",{address:ips[role]});}
  await success("one-way direct TCP UDP ICMP returns, reverse initiation denied");
  const through={...policy,mesh:[{id:"mandatory-via",src:"*",dst:"*",via:[`node:${ids.LAX}`]}]};
  await put(through);await success("one-way mandatory via TCP UDP ICMP returns, reverse initiation denied");
  for(const role of ["NYC","SHA"] as const){const proof=await helper(role,"execution-proof");assert(JSON.stringify(proof.paths).includes(ids.LAX));}
  await helper("SHA","agent-stop");await helper("SHA","agent-start");await success("same-identity destination restart requires fresh execution and flows");
  await put({...through,grants:[]});await ready();await deny("grant withdrawal denies both directions");
  await put(through);await success("restore one-way authorized path");
  await helper("LAX","stall",{mode:"all"});await delay(18_000);await deny("lease expiry fences while HTTP stalls");
  for(const role of ROLES)assert((await observe(role)).routes.every(noFallback));
  await helper("LAX","stall",{mode:"off"});await success("fresh activation after lease expiry");
  const asymmetric={...through,grants:[...policy.grants,{id:"independent-b",src:[`node:${ids.SHA}`],dst:[`node:${ids.NYC}`],bidirectional:false}],mesh:[
    {id:"a-via-c",src:`node:${ids.NYC}`,dst:`node:${ids.SHA}`,via:[`node:${ids.LAX}`]},
    {id:"b-direct-a",src:`node:${ids.SHA}`,dst:`node:${ids.NYC}`,via:[]},
  ]};
  await put(asymmetric);await ready();
  await until("asymmetric independent initiations",async()=>delivered(await one("NYC","SHA"))&&delivered(await one("SHA","NYC"))?true:false);
  await helper("SHA","udp-hold");await helper("NYC","udp-pending-start",{address:ips.SHA});
  await until("original A flow reached B through C",async()=>boolean((await helper("SHA","udp-observed")).received)?true:false,5_000);
  await helper("LAX","agent-stop");await helper("SHA","udp-release");
  assert(delivered(await one("SHA","NYC")),"B new grant must use its direct rule; its replies also stay direct");
  const held=await until("held original flow reply",async()=>{const value=await helper("NYC","udp-result");return value.done===true?value:false;},10_000);
  assert.equal(held.delivered,false,"A's existing flow reply cannot switch from C to B's direct initiation rule");
  await capture("asymmetric reverse rule cannot reroute an existing reply; independent B initiation remains direct");passed++;
  await helper("LAX","agent-start");await put(through);await success("restore one-way policy after asymmetric path proof");
  await helper("LAX","agent-stop");await delay(2_000);await deny("mandatory relay loss cannot use available direct A-B candidate");
  await helper("LAX","agent-start");await success("same-identity relay restart restores exact path");
  await revokeNode(baseUrl,admin,scope,{node_id:ids.LAX});await delay(5_000);await deny("revoked mandatory relay cannot fall back to direct A-B");
  status="PASS";
} catch(error) {
  evidence.push({failure:error instanceof Error?error.message:"unknown"});
  try{await capture("first-failure");evidence.push({errors:await Promise.all(ROLES.map(role=>helper(role,"agent-errors")))});}catch{}
  throw error;
} finally {
  for(const name of created.reverse())try{await docker(["rm","-f",name]);}catch{cleanupErrors.push(name);}
  if(networkCreated)try{await docker(["network","rm",network]);}catch{cleanupErrors.push(network);}
  await writeFile(join(artifacts,"result.json"),JSON.stringify({run,status,passed,evidence,cleanupErrors},null,2),{mode:0o600});
  console.log(`Stateful Mesh ${status} (${passed} checks): ${artifacts}`);
}
