import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { fstatSync } from "node:fs";
import { mkdtemp, lstat, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { command, parse, record, text, boolean, array } from "../managed-connections/local.ts";
import {
  listNetworksOperation, createEnrollmentTokenOperation, createManagedEnrollmentTokenOperation,
  getManagedListenersOperation, saveManagedListenerOperation, getConnectionsOperation,
  saveConnectionsOperation, savePolicyOperation, listNodesOperation, getConnectionRuntimesOperation,
  saveMeshAccessOperation,
} from "../../packages/control-client/src/generated/operations.gen.ts";
import type { ManagedListenerSettings } from "../../packages/control-client/src/generated/types.gen.ts";

const HERE=dirname(fileURLToPath(import.meta.url));const ROOT=resolve(HERE,"../..");
const TMP="/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode";
const OWNER="io.meshway.managed-transports";
process.umask(0o077);
const runId=`${Date.now().toString(36)}-${randomBytes(6).toString("hex")}`;
const prefix=`meshway-managed-vless-${runId}`;
const args=process.argv.slice(2);assert(args.length===0 || (args.length===2 && args[0]==="--image"));
const image=args[1]??`meshway-managed-vless:${runId}`;assert(/^meshway-managed-vless:[A-Za-z0-9._-]+$/.test(image));
const artifacts=await mkdtemp(join(TMP,`${prefix}-`));
const names={controller:`${prefix}-controller`,NYC:`${prefix}-nyc`,SHA:`${prefix}-sha`};
const network=`${prefix}-network`;const control=`${prefix}-control`;const containers:string[]=[];const ownedNetworks:string[]=[];
let interrupted=false;let cleaning=false;
let failureEvidence:(()=>Promise<void>)|undefined;
process.on("SIGINT",()=>{interrupted=true;});process.on("SIGTERM",()=>{interrupted=true;});
const report:{status:string;run_id:string;artifacts:string;step?:string;operation?:string;failure?:string;failure_operation?:string;evidence:object[];cleanup_errors:string[]}={status:"RUNNING",run_id:runId,artifacts,evidence:[],cleanup_errors:[]};
const save=()=>writeFile(join(artifacts,"result.json"),JSON.stringify(report,null,2)+"\n",{mode:0o600});
const docker=(args:string[],timeout=30_000,input?:string)=>{assert(!interrupted||cleaning);report.operation=`docker ${args[0]??""} ${args[1]??""}`;return command("docker",args,{cwd:ROOT,timeout,...(input===undefined?{}:{input})});};
const helper=async(role:keyof typeof names,action:string,data:object={})=>{report.step=`${role}/${action}`;return record(parse(await docker(["exec","-i",names[role],"python3","/private/harness.py",action],40_000,JSON.stringify(data))));};
async function until<T>(test:()=>Promise<T|false>,timeout=60_000):Promise<T>{const end=Date.now()+timeout;do{assert(!interrupted);const result=await test();if(result!==false)return result;await delay(500);}while(Date.now()<end);throw new Error("readiness deadline exceeded");}
try {
  const fd=Number(process.env.MESHWAY_MANAGED_TRANSPORTS_LOCK_FD);assert(Number.isSafeInteger(fd)&&fd>=3);
  const lock=process.env.MESHWAY_PERF_LOCK;assert(lock);const held=fstatSync(fd);const disk=await lstat(lock);assert(held.dev===disk.dev&&held.ino===disk.ino&&held.uid===process.getuid?.()&&(held.mode&0o077)===0);
  if(args.length===0)await docker(["build","-f","experiments/managed-connections/Dockerfile","-t",image,"."],1_200_000);
  const imageId=(await docker(["image","inspect",image,"--format","{{.Id}}"])).trim();assert(/^sha256:[0-9a-f]{64}$/.test(imageId));
  await docker(["network","create","--label",`${OWNER}=${runId}`,control]);ownedNetworks.push(control);
  await docker(["network","create","--internal","--label",`${OWNER}=${runId}`,network]);ownedNetworks.push(network);
  for(const role of ["controller","NYC","SHA"] as const){
    await docker(["create","--name",names[role],"--hostname",role.toLowerCase(),"--label",`${OWNER}=${runId}`,"--network",role==="controller"?control:network,"--network-alias",role==="controller"?"controller.mesh.test":`${role.toLowerCase()}.mesh.test`,"--cap-drop","ALL",...(role==="controller"?["--publish","127.0.0.1::8420"]:["--cap-add","NET_ADMIN","--device","/dev/net/tun:/dev/net/tun:rw"]),"--security-opt","no-new-privileges","--read-only","--tmpfs","/private:rw,nosuid,nodev,noexec,mode=0700,size=96m","--sysctl","net.ipv4.ip_forward=0","--sysctl","net.ipv6.conf.all.forwarding=0",...(role==="controller"?[]:["--sysctl","net.ipv6.conf.default.disable_ipv6=1"]),"--sysctl","net.ipv4.ping_group_range=0 2147483647",imageId]);containers.push(names[role]);
    if(role==="controller")await docker(["network","connect","--alias","controller.mesh.test",network,names[role]]);
    await docker(["start",names[role]]);
    for(const [source,target] of [[join(ROOT,"experiments/managed-dataplane/container.py"),"/private/dataplane.py"],[join(HERE,"container.py"),"/private/harness.py"]]){assert(source&&target);await docker(["exec","-i",names[role],"python3","-c","import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.write_bytes(sys.stdin.buffer.read()); p.chmod(0o600)",target],30_000,await readFile(source,"utf8"));}
  }
  await helper("controller","controller-start");
  const portLine=(await docker(["port",names.controller,"8420/tcp"])).trim();assert(/^127\.0\.0\.1:\d+$/.test(portLine));assert(![3000,8420,5173].includes(Number(portLine.split(":")[1])));const base=`http://${portLine}`;
  const admin=await until(async()=>{try{return text((await helper("controller","admin-token")).token);}catch{return false;}});
  const networks=await until(async()=>{try{return await listNetworksOperation(base,admin);}catch{return false;}});const networkId=networks.networks[0]?.network_id;assert(networkId);
  const initial:ManagedListenerSettings={id:"vless",node_id:"",enabled:true,transport:"vless",bind_address:"0.0.0.0",bind_port:18443,advertised_endpoints:[{host:"nyc.mesh.test",port:18443}],vless:{stream:"websocket",server_name:"meshway.test",websocket_path:"/meshway",websocket_host:"meshway.test"}};
  const enrollment=await createManagedEnrollmentTokenOperation(base,admin,networkId,{expires_in_seconds:600,initial_listener:initial});await helper("NYC","enroll",{name:"NYC",token:enrollment.token});
  const token=await createEnrollmentTokenOperation(base,admin,networkId,{expires_in_seconds:600});await helper("SHA","enroll",{name:"SHA",token:token.token});
  const nodes=(await listNodesOperation(base,admin,networkId)).nodes;const a=nodes.find(n=>n.name==="NYC");const b=nodes.find(n=>n.name==="SHA");assert(a&&b);
  let connections=await getConnectionsOperation(base,admin,networkId);
  connections=await saveConnectionsOperation(base,admin,networkId,{expected_revision:connections.revision,configuration:{listeners:connections.configuration.listeners,candidates:[{id:"reverse-dial",dialer_node_id:b.id,listener_id:"vless",endpoint:{host:"nyc.mesh.test",port:18443},priority:10,credential_ref:"managed.vless"}]}});
  const policy=await savePolicyOperation(base,admin,networkId,{expected_revision:connections.revision,policy:{default_action:"direct",rules:[]},source_jsonc:null});
  await saveMeshAccessOperation(base,admin,networkId,{expected_revision:policy.revision,policy:{default_action:"allow",rules:[]},source_jsonc:null});
  for(const role of ["NYC","SHA"] as const)await helper(role,"agent-start");
  failureEvidence=async()=>{report.evidence.push({phase:"failure-public-runtime",runtime:await getConnectionRuntimesOperation(base,admin,networkId)});for(const role of ["NYC","SHA"] as const)report.evidence.push({phase:"failure-public-local",role,local:await helper(role,"observe")});};
  const ready=async()=>{const runtime=await getConnectionRuntimesOperation(base,admin,networkId);return runtime.nodes.filter(n=>n.current&&!n.stale&&n.state==="applied"&&n.peers.some(p=>p.probe_fresh)).length===2?runtime:false;};
  report.evidence.push({phase:"websocket-tls-noise-authenticated",runtime:await until(ready)});
  const bodies={NYC:`meshway-tun:${runId}:NYC\n`,SHA:`meshway-tun:${runId}:SHA\n`};
  for(const [role,node] of [["NYC",a],["SHA",b]] as const){await helper(role,"http-start",{address:node.mesh_ipv4,body:bodies[role]});const material=await helper(role,"material-status");assert(boolean(material.journal_saved)&&boolean(material.journal_private)&&boolean(material.legacy_profiles_absent));}
  const bidirectional=async(phase:string)=>{const results:object[]=[];for(const [source,target,node] of [["NYC","SHA",b],["SHA","NYC",a]] as const){const proof=await until(async()=>{const value=await helper(source,"probe",{address:node.mesh_ipv4,body:bodies[target]});return boolean(value.http_ok)&&boolean(value.body_matches)&&value.ping_exit===0?value:false;});results.push({source,target,proof});}report.evidence.push({phase,results});await save();};
  await bidirectional("websocket-tls-vless-ipv4-two-directions");
  let listeners=await getManagedListenersOperation(base,admin,networkId);const original=listeners.listeners[0];assert(original);
  const settings={...original.settings,bind_port:18444,advertised_endpoints:[{host:"nyc.mesh.test",port:18444}],vless:{stream:"websocket" as const,server_name:"meshway.test",websocket_path:"/changed",websocket_host:"meshway.test"}};
  listeners=await saveManagedListenerOperation(base,admin,networkId,{expected_revision:listeners.revision,settings});
  connections=await getConnectionsOperation(base,admin,networkId);
  connections=await saveConnectionsOperation(base,admin,networkId,{expected_revision:connections.revision,configuration:{listeners:connections.configuration.listeners,candidates:connections.configuration.candidates.map(c=>({...c,endpoint:{host:"nyc.mesh.test",port:18444}}))}});
  report.evidence.push({phase:"port-path-change-current-snapshot",public:listeners,runtime:await until(ready)});await bidirectional("changed-port-and-path-two-directions");
  const bound=await docker(["exec",names.NYC,"ss","-H","-ltn"]);assert(bound.includes(":18444")&&!bound.includes(":18443"));
  await helper("SHA","agent-stop");await helper("SHA","agent-start");await until(ready);await bidirectional("restart-restores-managed-material");
  listeners=await getManagedListenersOperation(base,admin,networkId);const ws=listeners.listeners[0];assert(ws);
  listeners=await saveManagedListenerOperation(base,admin,networkId,{expected_revision:listeners.revision,settings:{...ws.settings,vless:{stream:"tcp",server_name:"meshway.test",websocket_path:null,websocket_host:null}}});
  assert.equal(listeners.listeners[0]?.certificate_sha256,original.certificate_sha256);await until(ready);await bidirectional("managed-tcp-tls-vless-two-directions");
  await helper("controller","controller-stop");await delay(18_000);
  const withdrawn=await helper("NYC","observe",{destinations:[b.mesh_ipv4]});const proof=record(withdrawn.dataplane_proof);
  const stoppedTraffic=await helper("NYC","probe",{address:b.mesh_ipv4,body:bodies.SHA});
  report.evidence.push({phase:"lease-expiry-controller-outage",withdrawn,stoppedTraffic});
  assert.equal(boolean(proof.tun_ready),false);
  assert(array(withdrawn.routes).every(r=>{const route=record(r);return route.kind==="guard"||(route.kind==="tun"&&route.device==="meshway0"&&boolean(proof.route_sink_retained));}));
  assert(!boolean(stoppedTraffic.http_ok)&&!boolean(stoppedTraffic.body_matches)&&stoppedTraffic.ping_exit!==0);
  assert(!(await docker(["exec",names.NYC,"ss","-H","-ltn"])).includes(":18444"));
  report.status="PASS";
} catch(error) {report.status="FAIL";report.failure=error instanceof Error?error.name:"unknown";if(report.operation)report.failure_operation=report.operation;try{await failureEvidence?.();}catch{}process.exitCode=1;} finally {
  cleaning=true;
  for(const name of containers.reverse()){try{assert.equal((await docker(["inspect",name,"--format",`{{index .Config.Labels "${OWNER}"}}`])).trim(),runId);await docker(["rm","-f",name]);}catch{report.cleanup_errors.push(name);}}
  for(const owned of ownedNetworks.reverse()){try{assert.equal((await docker(["network","inspect",owned,"--format",`{{index .Labels "${OWNER}"}}`])).trim(),runId);await docker(["network","rm",owned]);}catch{report.cleanup_errors.push(owned);}}
  if(report.cleanup_errors.length){report.status="FAIL";process.exitCode=1;}await save();console.log(JSON.stringify({status:report.status,artifacts,cleanup_errors:report.cleanup_errors}));
}
