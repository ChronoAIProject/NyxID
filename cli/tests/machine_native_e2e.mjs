// Run as root ONLY in an expendable native Linux guest, never an owner node.
// Requires the documented --separate-users browser/display prerequisites and ws.
// NYXID_TEST_CLI and NYXID_TEST_CUA_DRIVER select locally built/pinned binaries.
import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {createHash,createHmac,randomBytes,randomUUID} from 'node:crypto';
import {once} from 'node:events';
import http from 'node:http';
import https from 'node:https';
import fs from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {createRequire} from 'node:module';
import {contexts} from './machine_context_e2e.mjs';
import {selectBrowser} from './machine_native_browser.mjs';
const {WebSocketServer}=createRequire(import.meta.url)('ws');
assert(process.argv.includes('--disposable'),'requires explicit --disposable in an expendable Linux guest');
assert.equal(process.platform,'linux');assert.equal(process.getuid(),0);
const binary=process.env.NYXID_TEST_CLI,cua=process.env.NYXID_TEST_CUA_DRIVER;
assert(binary&&path.isAbsolute(binary),'NYXID_TEST_CLI must name the built CLI');
assert(cua&&path.isAbsolute(cua),'NYXID_TEST_CUA_DRIVER must name the pinned driver');
const {browserBinary,policyDirectory,browserExecutable}=selectBrowser();
// Setup must launch the very package whose policies this harness inspects.
process.env.NYXID_MACHINE_BROWSER=browserBinary;
const profileName=process.env.NYXID_TEST_NATIVE_PROFILE??`w94-${process.pid}`;
assert(/^[a-z][a-z0-9-]{0,15}$/.test(profileName),'safe unique native test profile');
const directory=`/var/lib/nyxid-machine/${profileName}/node`,workspace=`/srv/nyxid-${profileName}-workspace`;
assert(!(await fs.access(path.dirname(directory)).then(()=>true,()=>false)),'use a fresh native profile');
const root=await fs.mkdtemp(path.join(tmpdir(),'nyxid-machine-native-'));
// Browser identities import the public fixture certificate by its known path.
await fs.chmod(root,0o711);
console.log(`Native acceptance profile=${profileName} artifacts=${root}`);
for(const backup of [browserBinary+'-cold-test-real','/opt/nyxid/machine-browser/native-host-probe-test-real'])assert(!(await fs.access(backup).then(()=>true,()=>false)),`pre-existing test backup: ${backup}`);
const signing=randomBytes(32),nodeId=randomUUID();
const token=`nyx_nreg_${randomBytes(32).toString('hex')}`,auth=`nyx_nauth_${randomBytes(32).toString('hex')}`;
const conversationId=randomUUID(),turnId=randomUUID(),responses=new Map(),frames=[];
let socket,profile,child,receivedProfile=0,droppedMachineResultConnection;
const checkpoints=[];
function checkpoint(label){checkpoints.push(label);console.log(`PASS ${label}`);}
const server=http.createServer((_req,res)=>{res.writeHead(200,{'content-type':'application/json'});res.end('{"items":[]}');});
const wss=new WebSocketServer({server});
wss.on('connection',connection=>connection.on('message',(raw,binary)=>{
 if(binary){if(raw.subarray(0,4).toString()==='NYXM')frames.push({kind:raw[4],session:raw.subarray(6,22).toString('hex')});return;}
 const message=JSON.parse(raw);
 if(message.type==='register'){assert.equal(message.token,token);connection.send(JSON.stringify({type:'register_ok',node_id:nodeId,auth_token:auth,signing_secret:signing.toString('hex')}));}
 else if(message.type==='auth'){assert.equal(message.token,auth);socket=connection;connection.send(JSON.stringify({type:'auth_ok',heartbeat_interval_secs:10,capabilities:{proxy_binary_chunks:true}}));}
 else if(message.capabilities?.machine){if(profile)Object.assign(profile,message.capabilities.machine);else profile=message.capabilities.machine;receivedProfile++;}
 else if(message.type==='machine_result'){
  if(droppedMachineResultConnection===connection){droppedMachineResultConnection=undefined;return;}
  responses.get(message.request_id)?.(message.result);responses.delete(message.request_id);
 }else if(message.type==='machine_service_call'&&message.operation==='job_finished')connection.send(JSON.stringify({type:'machine_job_finished_ack',request_id:message.request_id}));
}));
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function waitFor(check,label,ms=45000){const end=Date.now()+ms;while(Date.now()<end){if(check())return;await delay(5);}throw new Error(`Timed out: ${label}`);}
function run(command,args){const r=spawnSync(command,args,{encoding:'utf8'});assert.equal(r.status,0,`${command}: ${r.stderr}`);return r.stdout;}
function canonical(value){if(Array.isArray(value))return `[${value.map(canonical).join(',')}]`;if(value&&typeof value==='object')return `{${Object.keys(value).sort().map(k=>JSON.stringify(k)+':'+canonical(value[k])).join(',')}}`;return JSON.stringify(value);}
function request(operation,parameters,authority,requestId){
 parameters={conversation_id:conversationId,turn_id:turnId,...parameters};
 if(operation==='exec')parameters={...parameters,runtime_id:profile.runtime_id};
 const r={type:'machine_request',request_id:requestId??randomUUID(),node_id:nodeId,operation,parameters,timestamp:Math.floor(Date.now()/1000),nonce:randomUUID()};
 const mac=createHmac('sha256',signing);
 if(authority){r.type='machine_request_v2';r.version=2;r.authority=authority;const version=Buffer.alloc(4);version.writeUInt32BE(2);mac.update(Buffer.from('nyxid.machine.request.v2\0')).update(createHash('sha256').update(canonical(authority)).digest()).update(version);}
 else mac.update(Buffer.from('nyxid.machine.request.v1\0'));
 const time=Buffer.alloc(8);time.writeBigInt64BE(BigInt(r.timestamp));
 for(const field of [r.request_id,nodeId,JSON.stringify(operation),createHash('sha256').update(canonical(parameters)).digest(),time,r.nonce]){const bytes=Buffer.isBuffer(field)?field:Buffer.from(field),length=Buffer.alloc(8);length.writeBigUInt64BE(BigInt(bytes.length));mac.update(length).update(bytes);}
 r.signature=mac.digest('hex');return r;
}
async function call(operation,parameters,authority,requestId,responseTimeoutMs=40000){
 const message=request(operation,parameters,authority,requestId);
 const response=new Promise((resolve,reject)=>{const timer=setTimeout(()=>{responses.delete(message.request_id);reject(new Error(`Timed out: ${operation}`));},responseTimeoutMs);responses.set(message.request_id,result=>{clearTimeout(timer);resolve(result);});});
 socket.send(JSON.stringify(message));return await response;
}
async function callWithReconnect(operation,parameters,authority,requestId){
 const message=request(operation,parameters,authority,requestId),oldSocket=socket;
 const response=new Promise((resolve,reject)=>{const timer=setTimeout(()=>{responses.delete(message.request_id);reject(new Error(`Timed out reconnect: ${operation}`));},40000);responses.set(message.request_id,result=>{clearTimeout(timer);resolve(result);});});
 droppedMachineResultConnection=oldSocket;oldSocket.send(JSON.stringify(message));await delay(30);oldSocket.terminate();
 await waitFor(()=>socket&&socket!==oldSocket&&socket.readyState===1,'operation reconnect',30000);
 socket.send(JSON.stringify(message));return await response;
}
function launch(){
 child=spawn(binary,['node','start','--config',directory],{detached:true,stdio:['ignore','pipe','pipe']});
 for(const stream of [child.stdout,child.stderr])stream.on('data',bytes=>daemonLog.write(bytes));
}
const daemonLog=await fs.open(`${root}/daemon.log`,'a',0o600).then(handle=>handle.createWriteStream());
async function stop(){
 if(child&&child.exitCode===null){const done=once(child,'exit');process.kill(-child.pid,'SIGTERM');await done;}
}
async function restart(){const before=receivedProfile;await stop();launch();await waitFor(()=>receivedProfile>before,'native restart profile',60000);assert.equal(profile.separation_evidence.version,4);assert.equal(profile.separation_evidence.credential_custody,true);assert(profile.separated.available);}
server.listen(0,'127.0.0.1');await once(server,'listening');
const heartbeat=setInterval(()=>{if(socket?.readyState===1)socket.send(JSON.stringify({type:'heartbeat_ping'}));},3000);
let website,browserSite;
try{
 const certificate=`${root}/tls.crt`;
 run('openssl',['req','-x509','-newkey','rsa:2048','-nodes','-keyout',`${root}/tls.key`,'-out',certificate,'-days','1','-subj','/CN=127.0.0.1','-addext','subjectAltName=IP:127.0.0.1']);
 website=https.createServer({key:await fs.readFile(`${root}/tls.key`),cert:await fs.readFile(certificate)},(_req,res)=>{res.setHeader('content-type','text/html');res.end('<!doctype html><title>NyxID sign-in test</title><form><input id="username" type="email" autofocus><input id="password" type="password"><button>Sign in</button></form>');});
 website.listen(0,'127.0.0.1');await once(website,'listening');
 browserSite=http.createServer((_req,res)=>{res.setHeader('content-type','text/html');res.end('<!doctype html><title>Machine browser fixture</title><h1>Project catalog</h1><input aria-label="Search projects"><button>Open project</button>');});
 browserSite.listen(0,'127.0.0.1');await once(browserSite,'listening');
 const setup=spawn(binary,['node','setup','--machine','--browser','--computer','--separate-users','--profile',profileName,'--root',workspace,'--separation','require-separated','--no-daemon','--cua-driver',cua,'--token',token,'--url',`ws://127.0.0.1:${server.address().port}/api/v1/nodes/ws`],{stdio:['ignore','pipe','pipe']});
 let setupOutput='';for(const stream of [setup.stdout,setup.stderr])stream.on('data',bytes=>{setupOutput+=bytes;});
 const [setupCode]=await once(setup,'exit');await fs.writeFile(`${root}/setup.log`,setupOutput.replaceAll(token,'[test-registration-token]'),{mode:0o600});
 assert.equal(setupCode,0,setupOutput.replaceAll(token,'[test-registration-token]'));
 checkpoint('fresh native --separate-users required setup and registration');
 launch();await waitFor(()=>profile,'native capability report',60000);
 assert.equal(profile.separation_choice,'require_separated');assert.equal(profile.separation_evidence.version,4);assert.equal(profile.separation_evidence.credential_custody,true);assert.equal(profile.separated.available,true,JSON.stringify(profile.separated));
 if(process.env.NYXID_TEST_REQUIRE_APPARMOR_USERNS==='1')assert.equal(profile.separation_evidence.apparmor_userns_restricted,true);
 console.log('EVIDENCE',JSON.stringify({separation_choice:profile.separation_choice,separation_evidence:profile.separation_evidence,separated:profile.separated}));
 checkpoint('positive v4 browser and credential-custody evidence');
 // A written policy is insufficient: prove the live negative control before
 // any context exists, then restore the exact protected inode in all cases.
 await stop();
 // Chromium reads policy files regardless of their extension. Move the inode
 // outside managed/, keeping it on the same filesystem for exact restoration.
 const policyFile=`${policyDirectory}/nyxid.json`,policyBackup=path.join(policyDirectory,'../nyxid.native-acceptance-disabled');
 assert(!(await fs.access(policyBackup).then(()=>true,()=>false)),'pre-existing policy test backup');
 await fs.rename(policyFile,policyBackup);
 try {
  const probe=run(binary,['node','setup','--probe-separation','--config',directory]);
  const unsupported=JSON.parse(probe);
  assert.equal(unsupported.version,4);assert.equal(unsupported.managed_browser_policy,false);
  assert.notEqual(unsupported.managed_browser_sandbox,true);
 } finally {await fs.rename(policyBackup,policyFile);}
 const beforePolicyRestart=receivedProfile;launch();await waitFor(()=>receivedProfile>beforePolicyRestart,'restored effective policy',60000);assert.equal(profile.separation_evidence.version,4);assert.equal(profile.separation_evidence.credential_custody,true);assert.equal(profile.separation_evidence.managed_browser_policy,true);assert.equal(profile.separated.available,true);
 checkpoint('missing secure policy refuses separation; restored policy is effective');
 await contexts({call,callWithReconnect,profile,conversationId,turnId,origin:`https://127.0.0.1:${website.address().port}`,browserUrl:`http://127.0.0.1:${browserSite.address().port}`,certificate,frames,native:{profile:profileName,workspace,restart,browserBinary},checkpoint});
 const journal=JSON.parse(await fs.readFile(`${directory}/machine-contexts.json`,'utf8'));
 const roles=Object.values(journal.contexts).map(c=>c.browsers[c.generation].secure_uid);
 const users=new Set(Object.values(journal.contexts).flatMap(c=>[c.command_uid,...Object.values(c.browsers).flatMap(b=>[b.secure_uid,b.dev_uid])]).map(uid=>`nyxc${uid}`));
 for(const file of ['/etc/subuid','/etc/subgid'])for(const line of (await fs.readFile(file,'utf8')).split('\n'))assert(!users.has(line.split(':')[0]),'context roles need no subordinate UID/GID grants');
 checkpoint('context accounts work without allocating subordinate UID/GID ranges');
 let rendered=0;const observed=new Set();
 for(const entry of await fs.readdir('/proc')){
  if(!/^\d+$/.test(entry))continue;
  try{
   const status=await fs.readFile(`/proc/${entry}/status`,'utf8'),uid=Number(status.match(/^Uid:\s+(\d+)/m)?.[1]);
   if(!roles.includes(uid))continue;
   const args=await fs.readFile(`/proc/${entry}/cmdline`,'utf8');if(!args.includes('--type=renderer'))continue;
   for(const field of ['Uid','Gid'])assert.deepEqual(status.match(new RegExp(`^${field}:\\s+(.+)$`,'m'))[1].trim().split(/\s+/).map(Number),[uid,uid,uid,uid]);
   assert.match(status,/^NoNewPrivs:\s+1$/m);assert.match(status,/^Seccomp:\s+2$/m);
   assert(status.match(/^NSpid:\s+(.+)$/m)[1].trim().split(/\s+/).length>=2,'renderer has nested PID namespace');
   const stat=await fs.readFile(`/proc/${entry}/stat`,'utf8'),fields=stat.slice(stat.lastIndexOf(') ')+2).split(/\s+/),parent=fields[2];
   const parentStatus=await fs.readFile(`/proc/${parent}/status`,'utf8');
   for(const field of ['Uid','Gid'])assert.deepEqual(parentStatus.match(new RegExp(`^${field}:\\s+(.+)$`,'m'))[1].trim().split(/\s+/).map(Number),[uid,uid,uid,uid]);
   assert(Number(status.match(/^Seccomp_filters:\s+(\d+)$/m)[1])>Number(parentStatus.match(/^Seccomp_filters:\s+(\d+)$/m)[1]),'renderer adds sandbox filters beyond its browser parent');
   assert.equal(await fs.readlink(`/proc/${entry}/exe`),browserExecutable);
   assert.equal(await fs.readlink(`/proc/${parent}/exe`),browserExecutable);
   for(const namespace of ['user','pid','net']) {
    const child=await fs.readlink(`/proc/${entry}/ns/${namespace}`);
    assert.notEqual(child,await fs.readlink(`/proc/${parent}/ns/${namespace}`));
    assert.notEqual(child,await fs.readlink(`/proc/self/ns/${namespace}`));
   }
   const recheck=await fs.readFile(`/proc/${entry}/stat`,'utf8');assert.equal(recheck.slice(recheck.lastIndexOf(') ')+2).split(/\s+/)[19],fields[19]);
   observed.add(uid);rendered++;
  }catch(error){if(error.code!=='ENOENT'&&error.code!=='ESRCH')throw error;}
 }
 assert.deepEqual([...observed].sort(),roles.sort(),'each restarted secure browser has a live sandboxed renderer');
 checkpoint(`live Chromium renderer identity, executable, namespace, NNP and additional seccomp checks (${observed.size} secure identities, ${rendered} renderers)`);
 console.log(`ACCEPTANCE ${checkpoints.length} passed / 0 failed / 0 skipped`);
}finally{
 await stop();clearInterval(heartbeat);
 for(const original of [browserBinary,'/opt/nyxid/machine-browser/native-host']){
  const backup=original+(original===browserBinary?'-cold-test-real':'-probe-test-real');
  if(await fs.access(backup).then(()=>true,()=>false))await fs.rename(backup,original);
 }

 try{const journal=JSON.parse(await fs.readFile(`${directory}/machine-contexts.json`,'utf8'));for(const context of Object.values(journal.contexts))for(const uid of [context.command_uid,...Object.values(context.browsers).flatMap(b=>[b.secure_uid,b.dev_uid])])spawnSync('pkill',['-KILL','-u',String(uid)]);}catch(error){if(error.code!=='ENOENT')console.error('cleanup:',error.message);}
 for(const user of [`nyxagent-${profileName}`,`nyxbrowser-${profileName}`,`nyxdev-${profileName}`])spawnSync('pkill',['-KILL','-u',user]);
 for(const client of wss.clients)client.terminate();wss.close();server.close();website?.close();browserSite?.close();daemonLog.end();
}
