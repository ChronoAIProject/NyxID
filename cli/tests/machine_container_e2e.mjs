// Run in the production machine image, with this directory and the `ws` package
// mounted read-only under /test. All credential values are generated in memory.
import assert from 'node:assert/strict';
import {contexts} from './machine_context_e2e.mjs';
import {spawn,spawnSync} from 'node:child_process';
import {createHash,createHmac,randomBytes,randomUUID} from 'node:crypto';
import {once} from 'node:events';
import http from 'node:http';
import https from 'node:https';
import fs from 'node:fs/promises';
import {createRequire} from 'node:module';
const {WebSocketServer}=createRequire(import.meta.url)('ws');
// Shared runners enforce sanity, not a workstation performance budget.
// Opt in explicitly on a quiet host; both modes always print measurements.
const strictBenchmark=process.env.NYXID_MACHINE_STRICT_BENCHMARK==='1';
const limits=strictBenchmark
 ? {fps:15,inputMs:100,takeoverMs:150,stopMs:1000,axWalkMs:1200}
 : {fps:8,inputMs:500,takeoverMs:2000,stopMs:5000,axWalkMs:5000};
console.log('Timing policy:',strictBenchmark?'strict benchmark':'CI sanity',JSON.stringify(limits));
const signing=randomBytes(32), nodeId=randomUUID(), auth=`nyx_nauth_${randomBytes(32).toString('hex')}`;
const token=`nyx_nreg_${randomBytes(32).toString('hex')}`;
const responses=new Map(), transfers=new Map(), output=[], frames=[], screenshotTargets=new Map(), screenshotWrites=[];
let socket, profile, child, website, browserSite;
let droppedMachineResultConnection;
let offlineUntil=0;
let conversationId=randomUUID(), turnId=randomUUID();
const values={username:`user-${randomBytes(12).toString('hex')}@example.test`,password:randomBytes(24).toString('base64url'),one_time_code:''};
const totpKey=randomBytes(20);
function totp(at=Date.now()){const counter=Buffer.alloc(8);counter.writeBigUInt64BE(BigInt(Math.floor(at/30000)));const digest=createHmac('sha1',totpKey).update(counter).digest(),offset=digest[19]&15;return String((digest.readUInt32BE(offset)&0x7fffffff)%1000000).padStart(6,'0');}
const hash=value=>createHash('sha256').update(value).digest('hex');
const siteEvents=[],results=[],gatewayCalls=[],gatewayCancels=[];
function run(command,args){const result=spawnSync(command,args,{encoding:'utf8'});assert.equal(result.status,0,`${command} failed: ${result.stderr}`);return result.stdout;}
const server=http.createServer((_req,res)=>{res.writeHead(200,{'content-type':'application/json'});res.end('{"items":[]}');});
const wss=new WebSocketServer({server});
wss.on('connection',connection=>connection.on('message',(raw,binary)=>{
  if(binary){
    if(raw.subarray(0,4).toString()==='NYXM'&&raw[4]===1&&raw.length>54&&raw.subarray(30,34).toString()==='NYXD'&&raw.readBigUInt64BE(46)===0n) {
      const id=raw.subarray(6,22).toString('hex'),target=screenshotTargets.get(id);
      if(target){screenshotTargets.delete(id);screenshotWrites.push(fs.writeFile(target,raw.subarray(54)));}
    }
    if(raw.subarray(0,4).toString()==='NYXM'&&raw[4]===10)gatewayCancels.push(raw.subarray(6,22).toString('hex'));
    if(raw.subarray(0,4).toString()==='NYXM')frames.push({at:performance.now(),bytes:raw.length,kind:raw[4],session:raw.subarray(6,22).toString('hex')});
    else {const stream=transfers.get(raw.subarray(0,36).toString());if(stream){stream.chunks.push(raw.subarray(36));stream.size+=raw.length-36;assert(stream.size<=5*1024*1024);}}
    return;
  }
  const message=JSON.parse(raw);
  if(message.type==='register'){
    assert.equal(message.token,token);
    connection.send(JSON.stringify({type:'register_ok',node_id:nodeId,auth_token:auth,signing_secret:signing.toString('hex')}));
  }else if(message.type==='auth'){
    if(Date.now()<offlineUntil){connection.close();return;}
    socket=connection;
    connection.send(JSON.stringify({type:'auth_ok',heartbeat_interval_secs:10,capabilities:{proxy_binary_chunks:true}}));
  }else if(message.capabilities?.machine){profile=message.capabilities.machine;}
  else if(message.type==='proxy_response_start'){assert.equal(message.status,200);}
  else if(message.type==='proxy_response_end'){const stream=transfers.get(message.request_id);if(stream){stream.resolve(Buffer.concat(stream.chunks));transfers.delete(message.request_id);}}
  else if(message.type==='proxy_error'){transfers.get(message.request_id)?.reject(new Error('file transfer refused'));transfers.delete(message.request_id);}
  else if(message.type==='machine_service_call'&&message.operation==='job_finished'){connection.send(JSON.stringify({type:'machine_job_finished_ack',request_id:message.request_id}));}
  else if(message.type==='machine_service_call'&&message.operation==='service_call'){
    gatewayCalls.push(message.request_id);
    connection.send(JSON.stringify({type:'machine_service_response',request_id:message.request_id,status:200,headers:[['content-type','text/plain']]}));
    const packet=Buffer.alloc(35);packet.write('NYXM');packet[4]=4;Buffer.from(message.request_id.replaceAll('-',''),'hex').copy(packet,6);packet.write('start',30);connection.send(packet);
  }
  else if(message.type==='machine_result'){
   if(droppedMachineResultConnection===connection){droppedMachineResultConnection=undefined;return;}
   responses.get(message.request_id)?.(message.result);responses.delete(message.request_id);
  }
}));
server.listen(0,'127.0.0.1');await once(server,'listening');
const heartbeat=setInterval(()=>{if(socket?.readyState===1)socket.send(JSON.stringify({type:'heartbeat_ping'}));},3000);
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function waitFor(check,label,ms=45000){const end=Date.now()+ms;while(Date.now()<end){if(check())return;await delay(5);}throw new Error(`Timed out: ${label}`);}
function canonical(value){if(Array.isArray(value))return `[${value.map(canonical).join(',')}]`;if(value && typeof value==='object')return `{${Object.keys(value).sort().map(k=>JSON.stringify(k)+':'+canonical(value[k])).join(',')}}`;return JSON.stringify(value);}
function request(operation,parameters,authority,requestId){
 parameters={conversation_id:conversationId,turn_id:turnId,...parameters};
 if(operation==='exec')parameters={...parameters,runtime_id:profile.runtime_id};
 const r={type:'machine_request',request_id:requestId??randomUUID(),node_id:nodeId,operation,parameters,timestamp:Math.floor(Date.now()/1000),nonce:randomUUID()};
 const mac=createHmac('sha256',signing);
 if(authority){
  r.type='machine_request_v2';r.version=2;r.authority=authority;
  const version=Buffer.alloc(4);version.writeUInt32BE(2);
  mac.update(Buffer.from('nyxid.machine.request.v2\0')).update(createHash('sha256').update(canonical(authority)).digest()).update(version);
 }else mac.update(Buffer.from('nyxid.machine.request.v1\0'));
 const time=Buffer.alloc(8);time.writeBigInt64BE(BigInt(r.timestamp));
 for(const field of [r.request_id,nodeId,JSON.stringify(operation),createHash('sha256').update(canonical(parameters)).digest(),time,r.nonce]){
  const bytes=Buffer.isBuffer(field)?field:Buffer.from(field),length=Buffer.alloc(8);length.writeBigUInt64BE(BigInt(bytes.length));mac.update(length).update(bytes);
 }
 r.signature=mac.digest('hex');return r;
}
async function call(operation,parameters,authority,requestId,responseTimeoutMs=40000){
 const message=request(operation,parameters,authority,requestId);
 const response=new Promise((resolve,reject)=>{const timeout=setTimeout(()=>{responses.delete(message.request_id);reject(new Error(`Timed out: ${operation}`));},responseTimeoutMs);responses.set(message.request_id,result=>{clearTimeout(timeout);resolve(result);});});
 socket.send(JSON.stringify(message));const result=await response;results.push(result);return result;
}
async function callWithReconnect(operation,parameters,authority,requestId){
 const message=request(operation,parameters,authority,requestId),oldSocket=socket;
 const response=new Promise((resolve,reject)=>{const timeout=setTimeout(()=>{responses.delete(message.request_id);reject(new Error(`Timed out: ${operation} after reconnect`));},40000);responses.set(message.request_id,result=>{clearTimeout(timeout);resolve(result);});});
 droppedMachineResultConnection=oldSocket;
 oldSocket.send(JSON.stringify(message));oldSocket.terminate();
 await waitFor(()=>socket&&socket!==oldSocket&&socket.readyState===1,'machine operation reconnect',30000);
 socket.send(JSON.stringify(message));const result=await response;results.push(result);return result;
}
async function transfer(operation,path,body=Buffer.alloc(0)){
 const message=request(operation,{path,max_bytes:5*1024*1024,size:body.length,sha256:hash(body)});message.type='proxy_upload';
 const response=new Promise((resolve,reject)=>transfers.set(message.request_id,{resolve,reject,chunks:[],size:0}));
 socket.send(JSON.stringify(message));
 let sequence=0;
 for(let offset=0;offset<=body.length;offset+=65536){
  const bytes=body.subarray(offset,offset+65536),end=offset>=body.length;
  const frame=Buffer.alloc(30+bytes.length);frame.write('NYXM');frame[4]=8;frame[5]=end?1:0;
  Buffer.from(message.request_id.replaceAll('-',''),'hex').copy(frame,6);frame.writeBigUInt64BE(BigInt(sequence++),22);bytes.copy(frame,30);socket.send(frame);
  if(end)break;
  if(offset+65536>=body.length)offset=body.length-65536;
 }
 let timer;
 try{return await Promise.race([response,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('stream transfer timeout')),20000);})]);}
 finally{clearTimeout(timer);transfers.delete(message.request_id);}
}
async function measureFrames(label,duration,action){
 const start=performance.now();let actions=0;
 while(performance.now()-start<duration){if(action){const result=await action();assert(!result.error,JSON.stringify(result));actions++;await delay(20);}else await delay(100);}
 const end=performance.now(),sample=frames.filter(f=>f.kind===1&&f.at>=start&&f.at<=end);
 return {scenario:label,seconds:(end-start)/1000,actions,frames:sample.length,fps:sample.length*1000/(end-start),bytes_per_second:sample.reduce((sum,f)=>sum+f.bytes,0)*1000/(end-start)};
}

try{
 const testDirectory=await fs.mkdtemp('/tmp/nyxid-browser-test-');await fs.chmod(testDirectory,0o755);
 run('openssl',['req','-x509','-newkey','rsa:2048','-nodes','-days','1','-keyout',`${testDirectory}/tls.key`,'-out',`${testDirectory}/tls.crt`,'-subj','/CN=NyxID local test','-addext','subjectAltName=IP:127.0.0.1','-addext','basicConstraints=critical,CA:TRUE']);
 run('runuser',['-u','browser','--','mkdir','-p','/home/browser/.pki/nssdb']);
 run('runuser',['-u','browser','--','certutil','-N','-d','sql:/home/browser/.pki/nssdb','--empty-password']);
 run('runuser',['-u','browser','--','certutil','-A','-d','sql:/home/browser/.pki/nssdb','-n','NyxID local test','-t','C,,','-i',`${testDirectory}/tls.crt`]);
 website=https.createServer({key:await fs.readFile(`${testDirectory}/tls.key`),cert:await fs.readFile(`${testDirectory}/tls.crt`)},(req,res)=>{
  if(req.method==='POST'){let body='';req.on('data',chunk=>{body+=chunk;});req.on('end',()=>{const event=JSON.parse(body);if(req.url==='/signin'){const valid=event.username===values.username&&event.password===values.password&&[totp(),totp(Date.now()-30000)].includes(event.one_time_code);siteEvents.push({signed_in:valid});res.end(valid?'Signed in':'Invalid login');}else{siteEvents.push(event);res.end('ok');}});return;}
  if(req.url==='/performance'){res.setHeader('content-type','text/html');res.end('<!doctype html><title>Desktop performance</title><style>body{font:24px sans-serif;background:#fafafa}textarea{width:900px;height:140px;font:24px monospace}</style><textarea autofocus></textarea>'+Array.from({length:100},(_,i)=>`<p style="background:hsl(${i*19} 50% 85%);padding:20px">Row ${i} — desktop streaming benchmark</p>`).join(''));return;}
  res.setHeader('content-type','text/html');
  res.end(`<!doctype html><title>NyxID sign-in test</title><style>body{font:24px sans-serif;padding:30px}input{display:block;margin:20px;font:24px sans-serif}</style><form><input id="username" type="email" autofocus><input id="password" type="password"><input id="one_time_code" type="text"><button>Sign in</button></form><script>
   document.addEventListener('input',async event=>{
    const bytes=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(event.target.value));
    await fetch('/result',{method:'POST',body:JSON.stringify({field:event.target.id,valueHash:Array.from(new Uint8Array(bytes),b=>b.toString(16).padStart(2,'0')).join(''),trusted:event.isTrusted})});
    if(event.target.id==='username')document.getElementById('password').focus();
    if(event.target.id==='password'){
     event.target.type='text';
     setTimeout(async()=>{await fetch('/result',{method:'POST',body:JSON.stringify({pinned:event.target.type==='password',copy_refused:!document.execCommand('copy')})});},0);
    }
   });
   document.querySelector('form').addEventListener('submit',async event=>{event.preventDefault();const result=await fetch('/signin',{method:'POST',body:JSON.stringify(Object.fromEntries(['username','password','one_time_code'].map(id=>[id,document.getElementById(id).value])))});document.body.textContent=await result.text();});
   window.addEventListener('load',()=>fetch('/result',{method:'POST',body:JSON.stringify({ready:true})}));
  </script>`);
 });
 website.listen(0,'127.0.0.1');await once(website,'listening');
 const origin=`https://127.0.0.1:${website.address().port}`;
 child=spawn('/usr/local/bin/nyxid-machine-entrypoint',['--machine','--browser','--computer'],{env:{...process.env,NYXID_NODE_TOKEN:token,NYXID_NODE_URL:`ws://127.0.0.1:${server.address().port}/api/v1/nodes/ws`},detached:true,stdio:['ignore','pipe','pipe']});
 for(const stream of [child.stdout,child.stderr])stream.on('data',bytes=>{output.push(bytes.toString());});
 await waitFor(()=>profile,'machine capabilities',60000);
 assert.equal(profile.browser_isolated,true);
 assert.equal(profile.commands_isolated,true);
 assert.equal(profile.computer_ready,true,'cua MCP must be ready');
 assert.equal(profile.saved_login_ready,true,'production signed extension and native host must connect');
 // Shell-only separated VM reports command isolation without a browser.
 for (const separated of [true,false]) {
  const dir=`${testDirectory}/${separated?'separated':'single-user'}`;
  await fs.mkdir(dir,{mode:0o700});
  const config=`[server]\nurl = "ws://127.0.0.1:9"\n[node]\nid = "isolation-test"\nauth_token_encrypted = ""\n[machine]\nshell = true\nfiles = true\ncomputer = false\nroots = ["/workspace"]\n${separated?'agent_user = "agent"\nbrowser_user = "browser"\n':''}`;
  await fs.writeFile(`${dir}/config.toml`,config,{mode:0o600});
  if(!separated){await fs.chown(dir,1000,1000);await fs.chown(`${dir}/config.toml`,1000,1000);}
  const args=['nyxid','node','machine','--config',dir,'status'];
  const result=separated?run(args[0],args.slice(1)):run('runuser',['-u','agent','--',...args]);
  const reported=JSON.parse(result);assert.equal(reported.commands_isolated,separated);assert.equal(reported.computer,false);
  if(separated){
   await fs.chmod(dir,0o755);
   const exposed=JSON.parse(run(args[0],args.slice(1)));
   assert.equal(exposed.commands_isolated,false,'configured agent user is insufficient when its access probe succeeds');
  }
 }
 // Production renderers must use nested user/PID namespaces and seccomp.
 const renderers=[];
 for(const pid of await fs.readdir('/proc')) {
  if(!/^\d+$/.test(pid))continue;
  const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split(/[\0\s]+/),()=>[]);
  if(!args.includes('--type=renderer'))continue;
  assert(!args.includes('--no-sandbox'));assert(!args.includes('--disable-setuid-sandbox'));
  const status=await fs.readFile(`/proc/${pid}/status`,'utf8');
  assert.match(status,/NoNewPrivs:\s+1/);assert.match(status,/Seccomp:\s+2/);
  assert(status.match(/NSpid:\s+([^\n]+)/)[1].trim().split(/\s+/).length>=2,'renderer has a nested PID namespace');
  const mapping=await fs.readFile(`/proc/${pid}/uid_map`,'utf8');
  assert.match(mapping,/\b1001\s+1\b/,'renderer namespace maps only the browser uid');
  renderers.push(pid);
 }
 assert(renderers.length>0,'a sandboxed production renderer must be running');
 const policy=JSON.parse(await fs.readFile('/etc/chromium/policies/managed/nyxid.json','utf8'));
 assert.equal(policy.DeveloperToolsAvailability,2);assert.equal(policy.RemoteDebuggingAllowed,false);
 assert.deepEqual(policy.URLBlocklist,['javascript:*','file://*']);assert.equal(policy.PasswordManagerEnabled,false);
 assert.equal(policy.ExtensionSettings[policy.ExtensionInstallForcelist[0].split(';')[0]].installation_mode,'force_installed');
 // Readiness requires the production extension's admin/non-disableable check.
 const debugProfile=`${testDirectory}/debug-refusal`;await fs.mkdir(debugProfile);await fs.chown(debugProfile,1001,1001);
 const debugProbe=spawn('runuser',['-u','browser','--','chromium','--headless=new',`--user-data-dir=${debugProfile}`,'--remote-debugging-port=0','about:blank'],{detached:true,stdio:['ignore','ignore','pipe']});
 let debugMessages='';debugProbe.stderr.on('data',bytes=>{debugMessages+=bytes.toString();});
 try{await waitFor(()=>/remote debugging.*(disallowed|disabled)|DevTools.*(disallowed|disabled)/i.test(debugMessages),'managed policy rejects DevTools',15000);assert(!debugMessages.includes('DevTools listening'));assert.equal(await fs.access(`${debugProfile}/DevToolsActivePort`).then(()=>true,()=>false),false);}
 finally{try{process.kill(-debugProbe.pid,'SIGTERM');}catch{}}
 const boundary=await call('exec',{job_id:randomUUID(),conversation_id:randomUUID(),command:`python3 - <<'CHECK'
import os,socket,ctypes,errno,platform,subprocess
assert 'NoNewPrivs:\t1' in open('/proc/self/status').read()
assert 'DBUS_SESSION_BUS_ADDRESS' not in os.environ and 'AT_SPI_BUS_ADDRESS' not in os.environ
libc=ctypes.CDLL(None,use_errno=True)
unshare,setns,clone=(97,268,220) if platform.machine()=='aarch64' else (272,308,56)
for number,arg,expected in [(unshare,0,errno.EPERM),(setns,-1,errno.EPERM),(435,0,errno.ENOSYS)]+[(clone,flag,errno.EPERM) for flag in [0x80,0x20000,0x2000000,0x4000000,0x8000000,0x10000000,0x20000000,0x40000000]]:
 assert libc.syscall(number,arg,0,0,0,0)==-1
 assert ctypes.get_errno()==expected,(number,ctypes.get_errno())
assert subprocess.run(['unshare','-Ur','true'],capture_output=True).returncode!=0
subprocess.run(['git','init','-q','/workspace/namespace-filter-git'],check=True)
subprocess.run(['git','-C','/workspace/namespace-filter-git','-c','user.name=Test','-c','user.email=test@example.test','commit','--allow-empty','-qm','works'],check=True)
for path in ['/etc/chromium/policies/managed/nyxid.json','/opt/nyxid/machine-browser/filler.crx','/etc/chromium/native-messaging-hosts/dev.nyxid.machine_filler.json']:
 assert not os.access(path,os.W_OK)
assert not os.access('/var/lib/nyxid-machine/desktop/browser-profile',os.R_OK)
assert not os.access('/var/lib/nyxid-machine-update',os.W_OK)
s=socket.socket(socket.AF_UNIX)
try: s.connect('/var/lib/nyxid-machine/desktop/browser-run/filler.sock')
except PermissionError: pass
else: raise AssertionError('agent reached supervisor socket')
CHECK`,cwd:'/workspace',services:[],timeout_secs:10});
 assert.equal(boundary.exit_code,0,JSON.stringify(boundary));
 const result=await call('exec',{job_id:randomUUID(),conversation_id:randomUUID(),command:'id -u; git --version',cwd:'/workspace',services:[],timeout_secs:10});
 assert.equal(result.exit_code,0,JSON.stringify(result));assert.match(result.stdout,/1000/);
 const transferBytes=randomBytes(2*1024*1024),transferStarted=performance.now();
 const written=JSON.parse((await transfer('save_attachment','/workspace/transfer.bin',transferBytes)).toString());
 assert.equal(written.sha256,hash(transferBytes));
 assert.equal(hash(await transfer('share_file','/workspace/transfer.bin')),hash(transferBytes));
 const transferMs=performance.now()-transferStarted;

 const desktop=await call('computer',{tool:'get_desktop_state',arguments:{max_image_dimension:1280}});
 assert.equal(desktop.isError,undefined,JSON.stringify(desktop).slice(0,300));
 assert(desktop.content?.some(item=>item.type==='image'),'cua must capture the real Xvfb desktop');
 const computer=(tool,arguments_)=>call('computer',{tool,arguments:tool.startsWith('clipboard_')?arguments_:{...arguments_,target:{kind:'desktop',display_id:'primary'}}});
 await computer('hotkey',{keys:['CTRL','L']});await computer('type_text',{text:origin});await computer('press_key',{key:'ENTER'});
 await waitFor(()=>siteEvents.some(e=>e.ready),'trusted local sign-in page');
 const mismatch=await call('fill_login',{field:'username',allowed_origins:['https://wrong.example.test'],value:values.username});
 assert.equal(mismatch.status,'refused');assert(!siteEvents.some(e=>e.field));
 const wrongField=await call('fill_login',{field:'password',allowed_origins:[origin],value:values.password});
 assert.equal(wrongField.status,'refused');assert(!siteEvents.some(e=>e.field));
 // Generate the code immediately before insertion. Like a real TOTP verifier,
 // the fixture accepts the preceding 30-second step if submission crosses it.
 for(const field of Object.keys(values)){
  if(field==='one_time_code')values[field]=totp();
  const value=values[field];
  if(field==='password')assert.notEqual((await computer('clipboard_write',{text:'nyxid-copy-sentinel'})).isError,true);
  const filled=await call('fill_login',{field,allowed_origins:[origin],value});
  assert.equal(filled.status,'filled',JSON.stringify(filled));
  await waitFor(()=>siteEvents.some(e=>e.field===field),'trusted input event');
  const event=siteEvents.find(e=>e.field===field);assert.equal(event.valueHash,hash(value));assert.equal(event.trusted,true);
  if(field==='password'){
   await waitFor(()=>siteEvents.some(e=>e.pinned),'password reveal pinning');
   await computer('hotkey',{keys:['CTRL','A']});await computer('hotkey',{keys:['CTRL','C']});
   const clipboard=await computer('clipboard_read',{include_text:true});assert(JSON.stringify(clipboard).includes('nyxid-copy-sentinel'),'a real copy gesture cannot replace the clipboard with the password');
   await computer('press_key',{key:'TAB'});
  }
 }
 assert(siteEvents.some(e=>e.pinned && e.copy_refused),'reveal and copy must be blocked');
 await computer('press_key',{key:'ENTER'});await waitFor(()=>siteEvents.some(e=>e.signed_in),'complete username/password/TOTP sign-in');
 await computer('hotkey',{keys:['CTRL','L']});await computer('type_text',{text:"javascript:fetch('/result',{method:'POST',body:JSON.stringify({javascript_executed:true})})"});await computer('press_key',{key:'ENTER'});await delay(500);
 assert(!siteEvents.some(e=>e.javascript_executed),'managed policy blocks javascript URLs');
 const privateClipboard=await computer('clipboard_write',{file_path:'/var/lib/nyxid-machine/desktop/browser-profile/Preferences'});
 assert(privateClipboard.error||privateClipboard.isError,'clipboard cannot read the browser profile');
 const encoded=Object.values(values).flatMap(value=>[value,Buffer.from(value).toString('base64'),Buffer.from(value).toString('base64url'),Buffer.from(value).toString('hex'),encodeURIComponent(value)]);
 const scrubbed=await call('exec',{job_id:randomUUID(),conversation_id:randomUUID(),command:`printf '%s' '${encoded.join('|')}'`,cwd:'/workspace',services:[],timeout_secs:10});
 assert.equal(scrubbed.exit_code,0);assert(scrubbed.stdout.includes('[redacted]'));
 for(const value of Object.values(values)){assert(!JSON.stringify(results).includes(value),'tool results never disclose saved-login values');assert(!output.join('').includes(value),'node logs never disclose saved-login values');}

 // A small realistic task: filter a catalog, choose a result, complete a form,
 // choose priority and submit. Each extension action also observes the page.
 const savedTasks=[];
 browserSite=http.createServer((req,res)=>{
  if(req.url.startsWith('/event?')) {siteEvents.push(Object.fromEntries(new URL(req.url,'http://fixture').searchParams));res.end('ok');return;}
  if(req.url==='/trusted') {res.setHeader('content-type','text/html');res.end(`<!doctype html><title>Trusted interactions</title><style>body{padding:30px;font:20px sans-serif}button,a,input{display:block;margin:20px}</style>
   <button onclick="if(event.isTrusted && navigator.userActivation.isActive){window.open('/popup');fetch('/event?popup=trusted')}">Open popup</button>
   <a target="_blank" href="/blank">New tab link</a><input type="file" aria-label="Choose file" onclick="fetch('/event?file='+event.isTrusted+'&activation='+navigator.userActivation.isActive)">
   <button id="covered" onclick="fetch('/event?covered=clicked')">Covered target</button><script>const b=document.querySelector('#covered'),r=b.getBoundingClientRect();const o=document.createElement('div');Object.assign(o.style,{position:'fixed',left:r.left+'px',top:r.top+'px',width:r.width+'px',height:r.height+'px',background:'pink',zIndex:9999});document.body.append(o)</script>`);return;}
  if(req.url==='/frames') {res.setHeader('content-type','text/html');res.end(`<!doctype html><title>Frame fixture</title><h1>Embedded form</h1><iframe title="Cross-origin form" src="http://localhost:${browserSite.address().port}/frame-form" width="900" height="450"></iframe>`);return;}
  if(req.url==='/frame-form') {res.setHeader('content-type','text/html');res.end(`<!doctype html><h2>Child form</h2><form onsubmit="event.preventDefault();fetch('/event?frame='+encodeURIComponent(document.querySelector('#entry').value));document.querySelector('h2').textContent='Child saved'"><label>Embedded name<input id="entry"></label><input type="password" aria-label="Embedded password" value="iframe-private-value"><button>Submit embedded</button></form>`);return;}
  if(req.url==='/large') {res.setHeader('content-type','text/html');res.end('<!doctype html><title>Large fixture</title>'+Array.from({length:500},(_,i)=>`<section><button aria-label="Row ${String(i).padStart(3,'0')}" role="group"><input aria-label="Child ${i}"></button></section>`).join(''));return;}
  if(['/popup','/blank'].includes(req.url)) {res.setHeader('content-type','text/html');res.end('<!doctype html><title>Opened trusted</title><h1>Opened</h1>');return;}
  if(req.url.startsWith('/saved?')){savedTasks.push(Object.fromEntries(new URL(req.url,'http://fixture').searchParams));res.end('saved');return;}
  res.setHeader('content-type','text/html');res.end(`<!doctype html><title>Machine browser fixture</title>
  <style>body{font:20px sans-serif;padding:30px}input,button,select{display:block;margin:16px}</style>
  <h1>Project catalog</h1><label>Search projects<input id="search" type="search" oninput="document.querySelector('#choose').hidden=!('atlas'.includes(this.value.toLowerCase()))"></label>
  <button id="choose" onclick="document.querySelector('#details').hidden=false">Choose Atlas</button>
  <form id="details" hidden onsubmit="event.preventDefault();document.querySelector('h1').textContent='Task saved';console.log('fixture saved');fetch('/saved?'+new URLSearchParams(new FormData(this)))">
  <label>Task title<input id="task" name="title"></label><label>Priority<select id="priority" name="priority"><option>Normal</option><option>High</option></select></label><button>Save task</button></form>
  <input type="password" aria-label="Protected password" value="fixture-password"><input autocomplete="one-time-code" aria-label="Protected code" value="123456">
  <script>console.log('fixture loaded');fetch('/loaded')</script>`);
 });
 browserSite.listen(0,'127.0.0.1');await once(browserSite,'listening');
 const browserUrl=`http://127.0.0.1:${browserSite.address().port}/`;
 const browser=async(action,args={},which='secure')=>{
  const result=await call('browser',{browser:which,action,...args});
  if(which==='dev'&&result.error)console.error('Developer browser error:',JSON.stringify(result.error));
  return result;
 };
 // Persisted-profile recovery: do not use a new user-data-dir for any case.
 const secureProfile='/var/lib/nyxid-machine/desktop/browser-profile';
 const secureSocket='/var/lib/nyxid-machine/desktop/browser-run/filler.sock';
 const extensionId=policy.ExtensionInstallForcelist[0].split(';')[0];
 async function securePids(){
  const pids=[];
  for(const pid of await fs.readdir('/proc')){
   if(!/^\d+$/.test(pid))continue;
   const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split('\0'),()=>[]);
   if(args.includes(`--user-data-dir=${secureProfile}`)&&!args.some(a=>a.startsWith('--type=')))pids.push(Number(pid));
  }
  return pids;
 }
 async function healthySecure(label){
  const before=siteEvents.length;
  const state=await browser('navigate',{url:origin});
  assert.equal(state.status,'ok',`${label}: ${JSON.stringify(state)}`);
  await waitFor(()=>siteEvents.slice(before).some(e=>e.ready),`${label} page ready`);
  for(const field of ['username','password']){
   const start=siteEvents.length;
   const filled=await call('fill_login',{field,allowed_origins:[origin],value:values[field]});
   assert.equal(filled.status,'filled',`${label} fill ${field}: ${JSON.stringify(filled)}`);
   await waitFor(()=>siteEvents.slice(start).some(e=>e.field===field&&e.valueHash===hash(values[field])),`${label} trusted fill`);
  }
  console.log(`Secure persisted profile: ${label} browser action + fill_login passed`);
 }
 const socketBefore=await fs.stat(secureSocket),pidsBefore=await securePids();
 const statusReport=JSON.parse(run('nyxid',['node','machine','--config','/var/lib/nyxid-machine/node','status']));
 assert.equal(statusReport.saved_login_ready,true);
 assert.equal((await fs.stat(secureSocket)).ino,socketBefore.ino,'status cannot replace the supervisor socket');
 assert.deepEqual(await securePids(),pidsBefore,'status cannot launch another Chromium');
 await healthySecure('read-only machine status');
 // Repeated idle native-host disconnects must preserve Chromium and its tabs.
 async function nativePids(){
  const pids=[];
  for(const pid of await fs.readdir('/proc')){
   if(!/^\d+$/.test(pid))continue;
   const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split('\0'),()=>[]);
   if(args.includes('machine-native-host'))pids.push(Number(pid));
  }
  return pids;
 }
 for(let cycle=0;cycle<5;cycle++){
  const killed=await nativePids();
  assert(killed.length>0,'connected native host exists');
  for(const pid of killed)process.kill(pid,'SIGKILL');
  // Let the extension reconnect without any request resetting its backoff.
  const deadline=Date.now()+10000;
  while(!(await nativePids()).some(pid=>!killed.includes(pid))){
   assert(Date.now()<deadline,'native host reconnects before supervisor repair');
   await delay(50);
  }
  await delay(250); // allow hello to reach the supervisor before the next kill
  assert.deepEqual(await securePids(),pidsBefore,'native reconnect must not relaunch Chromium');
 }
 const reconnected=await browser('snapshot');
 assert.equal(reconnected.status,'ok',JSON.stringify(reconnected));
 assert.deepEqual(await securePids(),pidsBefore,'first request after reconnect preserves Chromium');
 console.log('Repeated native reconnects preserve the secure Chromium PID: passed');
 for(const scenario of ['plain relaunch','package hash change','missing package']){
  // Freeze only the daemon while arranging the crash; no profile writes race Chromium.
  process.kill(child.pid,'SIGSTOP');
  try{
   for(const pid of await securePids())process.kill(pid,'SIGKILL');
   await delay(150);
   if(scenario==='package hash change')await fs.writeFile('/var/lib/nyxid-machine/desktop/browser-run/extension-package-sha256','old-package');
   if(scenario==='missing package')await fs.rm(`${secureProfile}/Default/Extensions/${extensionId}`,{recursive:true,force:true});
  } finally {process.kill(child.pid,'SIGCONT');}
  await healthySecure(scenario);
 }
 let observed=await browser('navigate',{url:browserUrl});
 assert.equal(observed.status,'ok',JSON.stringify(observed));
 assert.match(observed.snapshot.text,/Project catalog/);
 assert(!JSON.stringify(observed).includes('fixture-password'));assert(!JSON.stringify(observed).includes('123456'));
 const protectedField=observed.snapshot.elements.find(e=>e.label==='Protected password');
 assert.equal((await browser('type',{ref:protectedField.ref,text:'refused'})).status,'refused');
 // Navigation's DOM result can precede the AT-SPI title/tree update. Wait for
 // this fixture's window and content, never fall back to an unrelated window.
 let win,state;
 const axDeadline=performance.now()+15000;
 do {
  const windows=(await call('computer',{tool:'list_windows',arguments:{}})).structuredContent?.windows??[];
  win=windows.find(w=>JSON.stringify(w).includes('Machine browser fixture'));
  if(win)state=await call('computer',{tool:'get_window_state',arguments:{pid:win.pid,window_id:win.window_id,include_screenshot:false,timeout_ms:limits.axWalkMs}});
  if(state && JSON.stringify(state).includes('Project catalog'))break;
  await delay(100);
 }while(performance.now()<axDeadline);
 assert(state && !state.structuredContent?.degraded_reason,JSON.stringify(state)?.slice(0,1800));
 assert(JSON.stringify(state).includes('Project catalog'),'AX must include visible page text');
 const indexed=state.structuredContent.elements.find(e=>e.label?.includes('Choose Atlas'));
 assert(indexed?.element_token,'AX must expose an indexed interactive element');
 const clicked=await call('computer',{tool:'click',arguments:{element_token:indexed.element_token,target:{kind:'window',pid:win.pid,window_id:win.window_id},delivery_mode:'foreground'}});
 assert(!clicked.error && !clicked.isError,JSON.stringify(clicked));
 assert((await browser('snapshot')).snapshot.elements.some(e=>e.label==='Task title'),'element-indexed cua click reveals the form');
 // The same task over the general computer path: each action must be followed
 // by a separate AX observation. This is measured with AX fixed on this image,
 // so the comparison does not exaggerate the old image's blind-browser cost.
 let cuaCalls=0,cuaState;
 const cuaTaskStart=performance.now();
 async function cuaStep(tool,args={}){
  cuaCalls++;
  const result=tool==='get_window_state'
   ?await call('computer',{tool,arguments:{pid:win.pid,window_id:win.window_id,include_screenshot:false,timeout_ms:limits.axWalkMs}})
   :await computer(tool,args);
  assert(!result.error&&!result.isError,JSON.stringify(result));return result;
 }
 async function observe(){cuaState=await cuaStep('get_window_state');}
 async function clickLabel(label){
  let element=cuaState.structuredContent.elements.find(e=>e.label===label);
  const deadline=performance.now()+15000;
  while(!element?.element_token && performance.now()<deadline){
   await delay(100);await observe();
   element=cuaState.structuredContent.elements.find(e=>e.label===label);
  }
  assert(element?.element_token,`AX missing ${label}: ${JSON.stringify(cuaState)}`);
  cuaCalls++;
  const result=await call('computer',{tool:'click',arguments:{element_token:element.element_token,target:{kind:'window',pid:win.pid,window_id:win.window_id},delivery_mode:'foreground'}});
  assert(!result.error&&!result.isError,JSON.stringify(result));
 }
 await cuaStep('hotkey',{keys:['CTRL','L']});await cuaStep('type_text',{text:browserUrl});await cuaStep('press_key',{key:'ENTER'});await observe();
 await clickLabel('Search projects');await cuaStep('type_text',{text:'Atlas'});await observe();
 await clickLabel('Choose Atlas');await observe();
 await clickLabel('Task title');await cuaStep('type_text',{text:'Review machine integration'});await observe();
 await clickLabel('Priority');await cuaStep('press_key',{key:'END'});await cuaStep('press_key',{key:'ENTER'});await observe();
 await clickLabel('Save task');await observe();
 assert(JSON.stringify(cuaState).includes('Task saved'));
 await waitFor(()=>savedTasks.length===1,'cua task saved');
 assert.deepEqual(savedTasks[0],{title:'Review machine integration',priority:'High'});
 console.log('Cua browser task:',JSON.stringify({calls:cuaCalls,wall_ms:performance.now()-cuaTaskStart}));
 const taskStart=performance.now();let taskCalls=0;
 async function step(action,args={}){taskCalls++;observed=await browser(action,args);assert.equal(observed.status,'ok',JSON.stringify(observed));return observed;}
 const ref=label=>{const e=observed.snapshot.elements.find(e=>e.label===label);assert(e,`missing ${label}`);return e.ref;};
 await step('navigate',{url:browserUrl});
 await step('type',{ref:ref('Search projects'),text:'Atlas'});
 await step('click',{ref:ref('Choose Atlas')});
 await step('type',{ref:ref('Task title'),text:'Review machine integration'});
 await step('select',{ref:ref('Priority'),value:'High'});
 await step('click',{ref:ref('Save task')});
 assert.match(observed.snapshot.text,/Task saved/);
 await waitFor(()=>savedTasks.length===2,'browser task saved');
 assert.deepEqual(savedTasks[1],savedTasks[0]);
 console.log('Browser task:',JSON.stringify({calls:taskCalls,wall_ms:performance.now()-taskStart}));

 // Trusted input supplies activation for popups, new tabs and native choosers.
 const trusted=await browser('navigate',{url:browserUrl+'trusted'});
 const trustedTab=trusted.snapshot.tab_id;
 const trustedRef=label=>trusted.snapshot.elements.find(e=>e.label===label).ref;
 const clickPopup=await browser('click',{tab_id:trustedTab,ref:trustedRef('Open popup')});
 assert.equal(clickPopup.input_mode,'trusted');
 await waitFor(()=>siteEvents.some(e=>e.popup==='trusted'),'trusted popup activation');
 let tabs=(await browser('tabs')).snapshot.tabs;
 const popup=tabs.find(t=>t.url===browserUrl+'popup');assert(popup,'window.open created a real tab');
 await browser('tabs_close',{tab_id:popup.id});
 await browser('click',{tab_id:trustedTab,ref:trustedRef('New tab link')});
 tabs=(await browser('tabs')).snapshot.tabs;
 const blank=tabs.find(t=>t.url===browserUrl+'blank');assert(blank,'target=_blank opened a tab');
 await browser('tabs_close',{tab_id:blank.id});
 const overlay=await browser('click',{tab_id:trustedTab,ref:trustedRef('Covered target')});
 assert.equal(overlay.status,'refused',JSON.stringify(overlay));assert.match(overlay.reason,/overlay_mismatch/);
 assert(!siteEvents.some(e=>e.covered));
 await browser('click',{tab_id:trustedTab,ref:trustedRef('Choose file')});
 await waitFor(()=>siteEvents.some(e=>e.file==='true'&&e.activation==='true'),'file chooser trusted activation');
 const chooser=await call('computer',{tool:'list_windows',arguments:{}});
 assert.match(JSON.stringify(chooser),/Open File|Open files|Choose.*[Ff]ile|File.*[Uu]pload/,'native file chooser opened');
 await computer('press_key',{key:'ESC'});
 // Repeat new-document input to catch compositor/focus races.
 for(let round=0;round<5;round++){
 // Child frame refs are routed independently, including cross-origin documents.
 observed=await browser('navigate',{url:browserUrl+'frames'});
 for(let n=0;n<20&&!observed.snapshot.elements.some(e=>e.label==='Embedded name');n++){await delay(50);observed=await browser('snapshot');}
 assert(observed.snapshot.frames.length>=2,JSON.stringify(observed));
 const embedded=observed.snapshot.elements.find(e=>e.label==='Embedded name');assert(embedded);
 assert(!embedded.ref.startsWith('f0:'));
 const password=observed.snapshot.elements.find(e=>e.label==='Embedded password');
 assert.equal((await browser('type',{ref:password.ref,text:'refused'})).status,'refused');
 assert(!JSON.stringify(observed).includes('iframe-private-value'));
 observed=await browser('type',{ref:embedded.ref,text:`Frame works ${round}`});
 assert.equal(observed.status,'ok',JSON.stringify(observed));
 const submit=observed.snapshot.elements.find(e=>e.label==='Submit embedded');assert(submit,JSON.stringify(observed));
 const submitted=await browser('click',{ref:submit.ref});
 assert.equal(submitted.status,'ok',JSON.stringify(submitted));
 await waitFor(()=>siteEvents.filter(e=>e.frame!==undefined).length>round,'cross-origin form submitted');
 assert.equal(siteEvents.filter(e=>e.frame!==undefined).at(-1).frame,`Frame works ${round}`);
 }
 observed=await browser('navigate',{url:browserUrl+'large'});
 const allRefs=[];let offset=0;
 for(let n=0;n<100;n++){
  const page=await browser('snapshot',{query:{role:'group'},offset});assert.equal(page.status,'ok');
  assert(Buffer.byteLength(JSON.stringify(page))<9000);
  allRefs.push(...page.snapshot.elements.map(e=>e.ref));
  if(!page.snapshot.more.elements)break;
  assert(page.snapshot.next_offset>offset);offset=page.snapshot.next_offset;
 }
 assert.equal(allRefs.length,500);assert.equal(new Set(allRefs).size,500);
 const last=await browser('snapshot',{query:{label:'Row 499'}});
 assert.equal(last.snapshot.elements.length,1);
 const scoped=await browser('snapshot',{scope:last.snapshot.elements[0].ref});
 assert.deepEqual(scoped.snapshot.elements.map(e=>e.label),['Child 499']);
 console.log('Trusted input, popup, new tab, file chooser, overlay refusal, cross-origin frame and 500-row paging: passed');
 observed=await browser('navigate',{url:browserUrl});

 // Stop preempts a quiet 30s cua call, shell process group and gateway stream.
 const stopTimes={};
 for(const scenario of ['computer','exec','gateway']){
  let pending;
  if(scenario==='computer'){
   for(const pid of await fs.readdir('/proc')){
    if(!/^\d+$/.test(pid))continue;
    const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split(/[\0\s]+/),()=>[]);
    if(args[0]==='/opt/nyxid/cua/cua-driver'&&args.includes('mcp'))process.kill(Number(pid),'SIGSTOP');
   }
   pending=call('computer',{tool:'get_desktop_state',arguments:{}});
  }else{
   const command=scenario==='exec'?'sleep 30':'curl --no-buffer --silent --header "Authorization: Bearer $NYXID_GATEWAY_TOKEN" "$NYXID_GATEWAY_URL/s/fixture/stream"';
   pending=call('exec',{job_id:randomUUID(),command,services:[],timeout_secs:30});
   if(scenario==='gateway')await waitFor(()=>gatewayCalls.length>0,'streaming gateway started');
  }
  await delay(50);
  const started=performance.now();
  assert.equal((await call('cancel',{conversation_id:conversationId,turn_id:turnId})).stopped,true);
  const stopped=await pending;
  assert.equal(stopped.error.code,12418,JSON.stringify(stopped));
  if(scenario==='gateway')await waitFor(()=>gatewayCancels.includes(gatewayCalls[0].replaceAll('-','')),'gateway cancellation',1000);
  stopTimes[scenario]=performance.now()-started;
  assert(stopTimes[scenario]<limits.stopMs,`${scenario} stop exceeded ${limits.stopMs} ms`);
  assert.equal((await call('computer',{tool:'get_desktop_state',arguments:{}})).error.code,12418,'late calls from stopped turn refused');
  turnId=randomUUID();
  const recovered=await call('computer',{tool:'list_windows',arguments:{}});
  assert(!recovered.error&&!recovered.isError,JSON.stringify(recovered));
 }
 console.log('Stop latency:',JSON.stringify(stopTimes));
 // Multiple driver crashes recover automatically, with no three/minute lockout.
 for(const killDelay of [0,10,100,0,10,100]){
  let killed=0;
  for(const pid of await fs.readdir('/proc')){
   if(!/^\d+$/.test(pid))continue;
   const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split(/[\0\s]+/),()=>[]);
   if(args[0]==='/opt/nyxid/cua/cua-driver'&&args.includes('mcp')){process.kill(Number(pid),'SIGKILL');killed++;}
  }
  assert(killed>0,'crash test must kill a live cua session');
  await delay(killDelay);
  const restarting=await call('computer',{tool:'list_windows',arguments:{}});
  assert.equal(restarting.error?.code,12414,`cua crash after ${killDelay} ms: ${JSON.stringify(restarting)}`);assert(restarting.error.retry_after_ms>0);
  await delay(restarting.error.retry_after_ms);
  const recovered=await call('computer',{tool:'list_windows',arguments:{}});
  assert(!recovered.error&&!recovered.isError,`cua recovery after ${killDelay} ms: ${JSON.stringify(recovered)}`);
  assert(profile.computer_ready&&profile.computer_tools.includes('get_window_state'));
  console.log(`Cua crash/recovery at ${killDelay} ms: passed`);
 }
 // Real startup failures must be diagnosable, never generic 12407, and retryable.
 const chromiumBackup='/usr/bin/chromium.nyxid-e2e-backup';
 await fs.rename('/usr/bin/chromium',chromiumBackup);
 try {
  let unavailable=await browser('navigate',{url:browserUrl},'dev');
  assert.equal(unavailable.error?.code,12413,JSON.stringify(unavailable));
  assert.match(unavailable.error.message,/cannot start developer browser/);
  await fs.writeFile('/usr/bin/chromium',"#!/bin/sh\necho 'https://stderr-secret-fixture/ cookie=stderr-secret-fixture' >&2\nexit 42\n",{mode:0o755});
  unavailable=await browser('navigate',{url:browserUrl},'dev');
  assert.equal(unavailable.error?.code,12413,JSON.stringify(unavailable));
  assert.match(unavailable.error.message,/pipe|exited/);
  assert(!output.join('').includes('stderr-secret-fixture'),'browser diagnostics must not expose child stderr');
 } finally {await fs.rename(chromiumBackup,'/usr/bin/chromium');}
 let dev=await browser('navigate',{url:browserUrl},'dev');assert.equal(dev.status,'ok',JSON.stringify(dev));
 dev=await browser('navigate',{url:browserUrl+'frames'},'dev');
 let devName=dev.snapshot.elements.find(e=>e.label==='Embedded name');
 for(let n=0;n<20&&!devName;n++){await delay(50);dev=await browser('snapshot',{},'dev');devName=dev.snapshot.elements.find(e=>e.label==='Embedded name');}
 assert(devName,JSON.stringify(dev));
 const devPassword=dev.snapshot.elements.find(e=>e.label==='Embedded password');
 assert.equal((await browser('type',{ref:devPassword.ref,text:'refused'},'dev')).status,'refused');
 dev=await browser('type',{ref:devName.ref,text:'Dev frame works'},'dev');
 assert.equal(dev.input_mode,'trusted',JSON.stringify(dev));
 const devSubmit=dev.snapshot.elements.find(e=>e.label==='Submit embedded');assert(devSubmit,JSON.stringify(dev));
 const devSubmitted=await browser('click',{ref:devSubmit.ref},'dev');
 assert.equal(devSubmitted.status,'ok',JSON.stringify(devSubmitted));
 await waitFor(()=>siteEvents.some(e=>e.frame==='Dev frame works'),'dev cross-origin form submitted');
 assert.equal(siteEvents.filter(e=>e.frame!==undefined).at(-1).frame,'Dev frame works');
 dev=await browser('navigate',{url:browserUrl},'dev');
 dev=await browser('evaluate',{expression:"document.title + ': ' + (21 * 2)"},'dev');assert.equal(dev.evaluation.result.value,'Machine browser fixture: 42');
 assert.equal((await browser('evaluate',{expression:'1+1'})).error.code,12407,'secure browser never evaluates scripts');
 assert.equal((await call('fill_login',{browser:'dev',field:'password',allowed_origins:[browserUrl],value:'never-type-this'})).error.code,12413);
 const fileRefused=await browser('evaluate',{expression:"fetch('file:///var/lib/nyxid-machine/desktop/browser-profile/Default/Cookies').then(()=>false,()=>true)"},'dev');
 assert.equal(fileRefused.evaluation.result.value,true,'dev browser cannot fetch secure profile');
 assert.notEqual((await browser('navigate',{url:'file:///etc/passwd'},'dev')).status,'ok');
 dev=await browser('navigate',{url:browserUrl},'dev');assert.equal(dev.status,'ok',JSON.stringify(dev));
 assert((await browser('console',{},'dev')).console.some(row=>JSON.stringify(row).includes('fixture loaded')));
 assert((await browser('network',{},'dev')).network.some(row=>row.url?.endsWith('/loaded')));
 assert((await browser('screenshot',{},'dev')).content.some(c=>c.type==='image'&&c.mimeType==='image/jpeg'));
 const securePath='/var/lib/nyxid-machine/desktop/browser-profile';
 assert.notEqual(spawnSync('runuser',['-u','devbrowser','--','cat',`${securePath}/Default/Cookies`]).status,0);
 // Direct container launch relies on distinct UIDs and the existing Chromium
 // sandbox, not bwrap mounts. Exercise actual filesystem/socket operations.
 const devBoundary=spawnSync('runuser',['-u','devbrowser','--','python3','-c',`
import os,socket,json
for path in ['/var/lib/nyxid-machine/desktop/browser-profile','/var/lib/nyxid-machine/node','/home/browser']:
 try: os.listdir(path)
 except PermissionError: pass
 else: raise AssertionError('devbrowser read protected directory: '+path)
for path in ['/var/lib/nyxid-machine/desktop/browser-profile/dev-write','/var/lib/nyxid-machine/node/dev-write','/workspace/dev-write','/var/lib/nyxid-machine-update/dev-write']:
 try: open(path,'w')
 except PermissionError: pass
 else: raise AssertionError('devbrowser wrote protected path: '+path)
for path in ['/var/lib/nyxid-machine/node/config.toml','/etc/chromium/policies/managed/nyxid.json','/etc/chromium/native-messaging-hosts/dev.nyxid.machine_filler.json','/opt/nyxid/machine-browser/native-host','/opt/nyxid/machine-browser/nyxid-native-host']:
 try: open(path,'rb')
 except PermissionError: pass
 else: raise AssertionError('devbrowser read protected file: '+path)
s=socket.socket(socket.AF_UNIX)
try: s.connect('/var/lib/nyxid-machine/desktop/browser-run/filler.sock')
except PermissionError: pass
else: raise AssertionError('devbrowser reached saved-login native host')
p='/etc/chromium/policies/managed/nyxid-dev.json'
assert not os.access(p,os.W_OK)
policy=json.load(open(p));assert policy['RemoteDebuggingAllowed'] and policy['NativeMessagingBlocklist']==['*']
assert policy['URLBlocklist']==['file://*'] and 'ExtensionInstallForcelist' not in policy
`],{encoding:'utf8'});
 assert.equal(devBoundary.status,0,devBoundary.stderr);
 assert.notEqual(spawnSync('runuser',['-u','browser','--','cat','/etc/chromium/policies/managed/nyxid-dev.json']).status,0,'secure browser cannot read developer policy');
 const devRenderers=[];
 for(const pid of await fs.readdir('/proc')) {
  if(!/^\d+$/.test(pid))continue;
  const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split(/[\0\s]+/),()=>[]);
  if(!args.includes('--type=renderer'))continue;
  const mapping=await fs.readFile(`/proc/${pid}/uid_map`,'utf8');
  if(!/\b1002\s+1\b/.test(mapping))continue;
  const status=await fs.readFile(`/proc/${pid}/status`,'utf8');
  assert.match(status,/NoNewPrivs:\s+1/);assert.match(status,/Seccomp:\s+2/);
  assert(status.match(/NSpid:\s+([^\n]+)/)[1].trim().split(/\s+/).length>=2);
  devRenderers.push(pid);
 }
 assert(devRenderers.length>0,'developer renderer retains Chromium namespace/seccomp sandbox');
 console.log('Developer UID, policy, credential/native-host boundaries and renderer sandbox: passed');
 assert.equal(await fs.access('/var/lib/nyxid-machine/desktop/dev-browser-profile/DevToolsActivePort').then(()=>true,()=>false),false);
 // X11 clients must not share trust across these browser users.
 for(const [user,display,authority] of [['devbrowser',':99','/var/lib/nyxid-machine/desktop/Xauthority'],['browser',':100','/home/devbrowser/.Xauthority']]) {
  const denied=spawnSync('runuser',['-u',user,'--','env',`DISPLAY=${display}`,`XAUTHORITY=${authority}`,'xdpyinfo'],{encoding:'utf8'});
  assert.notEqual(denied.status,0);assert.match(denied.stderr,/authoriz|authentication|unable to open display/i);
 }
 const devSession=randomUUID();
 if(process.env.NYXID_TEST_SCREENSHOT_DIR)screenshotTargets.set(devSession.replaceAll('-',''),process.env.NYXID_TEST_SCREENSHOT_DIR+'/dev-fixture.jpg');
 assert.equal((await call('desktop_open',{display:'dev',session_id:devSession})).streaming,true);
 await waitFor(()=>frames.some(f=>f.kind===1&&f.session===devSession.replaceAll('-','')),'dev display stream');
 await call('desktop_control',{display:'dev',session_id:devSession,viewer_id:'dev-owner',owner:true,revision:1});
 assert.equal((await browser('snapshot',{},'dev')).error.code,12408);
 assert.equal((await browser('snapshot')).status,'ok','secure display remains independently usable');
 await call('desktop_control',{display:'dev',session_id:devSession,viewer_id:'dev-owner',owner:false,revision:2});
 await call('desktop_close',{display:'dev',session_id:devSession});
 console.log('Independent display streams and mutual X authentication refusal: passed');
 // Bring the secure browser back to the visible foreground for cua/desktop.
 await browser('tabs_switch',{tab_id:observed.snapshot.tab_id});
 const session=randomUUID();
 if(process.env.NYXID_TEST_SCREENSHOT_DIR)screenshotTargets.set(session.replaceAll('-',''),process.env.NYXID_TEST_SCREENSHOT_DIR+'/secure-fixture.jpg');
 await call('desktop_open',{session_id:session});
 await waitFor(()=>frames.some(f=>f.kind===1&&f.session===session.replaceAll('-','')),'secure display frame');
 const agentDrivers=[];
 for(const pid of await fs.readdir('/proc')) {
  if(!/^\d+$/.test(pid))continue;
  const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split(/[\0\s]+/),()=>[]);
  if(args[0]==='/opt/nyxid/cua/cua-driver'&&args.includes('mcp'))agentDrivers.push(Number(pid));
 }
 assert(agentDrivers.length>0);
 for(const pid of agentDrivers)process.kill(pid,'SIGSTOP');
 const slowComputer=call('computer',{tool:'get_desktop_state',arguments:{}});
 // A 5 MiB write is deliberately stalled after 4 MiB. The worker is live,
 // blocked on input, and must be killed without delaying the owner's takeover.
 const pendingBytes=randomBytes(5*1024*1024);
 const large=request('save_attachment',{path:'/workspace/preempted.bin',max_bytes:pendingBytes.length,size:pendingBytes.length,sha256:hash(pendingBytes)});
 large.type='proxy_upload';
 const interrupted=new Promise(resolve=>transfers.set(large.request_id,{resolve:()=>resolve(false),reject:()=>resolve(true),chunks:[],size:0}));
 socket.send(JSON.stringify(large));
 for(let offset=0;offset<4*1024*1024;offset+=65536){
  const packet=Buffer.alloc(30+65536);packet.write('NYXM');packet[4]=8;
  Buffer.from(large.request_id.replaceAll('-',''),'hex').copy(packet,6);packet.writeBigUInt64BE(BigInt(offset/65536),22);pendingBytes.copy(packet,30,offset,offset+65536);socket.send(packet);
 }
 await delay(50);
 const takeoverStart=performance.now();
 const taken=await call('desktop_control',{session_id:session,viewer_id:'test-owner',owner:true,revision:1});
 const takeoverMs=performance.now()-takeoverStart;
 assert(takeoverMs<=limits.takeoverMs,`owner takeover took ${takeoverMs} ms (ceiling ${limits.takeoverMs} ms)`);
 assert.equal((await slowComputer).error.code,12408,'late cua result discarded');
 assert.equal(await interrupted,true,'in-flight large file worker cancelled');
 for(const pid of agentDrivers){
  const deadline=performance.now()+limits.stopMs;
  while(await fs.access(`/proc/${pid}`).then(()=>true,()=>false)){
   assert(performance.now()<deadline,'agent cua session killed and reaped');
   await delay(5);
  }
 }
 assert.equal(taken.controller,'owner');
 const denied=await call('exec',{job_id:randomUUID(),conversation_id:randomUUID(),command:'true',services:[]});
 assert.equal(denied.error.code,12408);
 for(const [operation,parameters]of [['read_file',{path:'/workspace/transfer.bin'}],['computer',{tool:'get_desktop_state',arguments:{}}]])assert.equal((await call(operation,parameters)).error.code,12408);
 const ownerSecret=randomBytes(20).toString('hex');
 await call('desktop_input',{session_id:session,viewer_id:'test-owner',revision:1,tool:'type_text',arguments:{text:ownerSecret}});
 assert(!JSON.stringify(results).includes(ownerSecret));assert(!output.join('').includes(ownerSecret));
 await call('desktop_control',{session_id:session,viewer_id:'test-owner',owner:false,revision:2});
 assert.equal((await call('exec',{job_id:randomUUID(),conversation_id:randomUUID(),command:'true',services:[]})).exit_code,0);
 assert(!(await fs.readdir('/workspace')).some(name=>name.startsWith('.nyxid-')||name==='preempted.bin'),'cancelled atomic writes leave no file or temporary file');

 await computer('hotkey',{keys:['CTRL','L']});await computer('type_text',{text:origin+'/performance'});await computer('press_key',{key:'ENTER'});await delay(1000);
 const cuaSamples={click:[],type_text:[],get_window_state:[]};
 let axSample;
 for(let n=0;n<10;n++)for(const tool of Object.keys(cuaSamples)){
  const started=performance.now();
  const result=tool==='get_window_state'
   ?await call('computer',{tool,arguments:{pid:win.pid,window_id:win.window_id,include_screenshot:false,timeout_ms:limits.axWalkMs}})
   :await computer(tool,tool==='click'?{x:200,y:170,delivery_mode:'foreground'}:{text:'sample '});
  assert(!result.error&&!result.isError,JSON.stringify(result));
  cuaSamples[tool].push(performance.now()-started);
  if(tool==='get_window_state')axSample=result;
 }
 assert(!axSample.structuredContent.degraded_reason);
 assert(!JSON.stringify(axSample).includes('unsupported command-line flag'),'no Chromium warning infobar');
 assert(!output.join('').includes('could not activate the persistent AT-SPI listener'));
 console.log('AX element count:',axSample.structuredContent.elements.length);
 for(const [tool,ms]of Object.entries(cuaSamples)){ms.sort((a,b)=>a-b);console.log(`${tool}: p50 ${ms[4].toFixed(2)} ms; p95 ${ms[9].toFixed(2)} ms`);}
 await call('desktop_control',{session_id:session,viewer_id:'test-owner',owner:true,revision:3});
 const input=(tool,args)=>call('desktop_input',{session_id:session,viewer_id:'test-owner',revision:3,tool,arguments:args});
 await input('click',{x:1150,y:600,delivery_mode:'foreground'});await delay(2500);
 const performanceSamples=[await measureFrames('idle',5000)];
 await input('click',{x:200,y:160,delivery_mode:'foreground'});
 performanceSamples.push(await measureFrames('typing',5000,()=>input('type_text',{text:'benchmark '})));
 // Continuous acknowledged scrolling measures changed-frame capacity; adding a
 // 200ms pause after each driver action would cap the source below two updates/s.
 // Reverse before reaching the page edge so an idle bottom is not measured as
 // a scrolling workload. Keep action counts beside frame counts for diagnosis.
 let scrolls=0;
 performanceSamples.push(await measureFrames('scrolling',5000,()=>input('scroll',{x:1150,y:650,direction:Math.floor(scrolls++/6)%2?'up':'down',amount:3,by:'line'})));
 const latencies=[];
 for(let n=0;n<10;n++){
  const start=performance.now(),prior=frames.length;
  await input('scroll',{x:1150,y:650,direction:n%2?'up':'down',amount:3,by:'line'});
  await waitFor(()=>frames.slice(prior).some(f=>f.kind===1),'updated frame',5000);
  latencies.push(frames.slice(prior).find(f=>f.kind===1).at-start);
 }
 latencies.sort((a,b)=>a-b);
 console.log('Desktop measurements:',JSON.stringify({scenarios:performanceSamples,input_to_frame_ms:{p50:latencies[4],p95:latencies[9]}}));
 for(const sample of performanceSamples.filter(s=>s.scenario!=='idle'))assert(sample.fps>=limits.fps,`${sample.scenario}: ${sample.fps} fps below ${limits.fps}`);
 assert(performanceSamples[0].bytes_per_second<1024,'idle bandwidth should be near zero');
 assert(latencies[9]<=limits.inputMs,`input-to-frame p95 ${latencies[9]} ms exceeds ${limits.inputMs} ms`);
 await call('desktop_control',{session_id:session,viewer_id:'test-owner',owner:false,revision:4});
 // Enroll only after the legacy regression matrix: v1 remains functional until
 // an explicit restricted assignment is installed, then cannot widen it.
 assert(profile.authority_versions.includes(2));
 const identity={require_v2:true,context_id:randomUUID(),generation:1,mode:'shared_legacy',agent_id:randomUUID(),owner_id:randomUUID(),actor_id:randomUUID(),group_id:null,runtime_id:profile.runtime_id,conversation_id:conversationId,turn_id:turnId,revision:1,capabilities:{shell:true,files:false,browser:false,computer:false,developer_browser:false}};
 const authority=(changes={})=>({...identity,lease_id:randomUUID(),expires_at_ms:Date.now()+45000,...changes});
 assert.equal((await call('read_file',{path:'/workspace/transfer.bin'},authority())).error.code,12420,'signed file denial');
 const leased=authority(),leasedJob=randomUUID();
 const startedLease=await call('exec',{job_id:leasedJob,command:'sleep 60',background:true,services:[]},leased);
 assert.equal(startedLease.job_id,leasedJob,JSON.stringify(startedLease));
 const runningLease=await call('job',{job_id:leasedJob},authority());
 assert.equal(runningLease.status,'running',JSON.stringify(runningLease));
 assert.equal((await call('exec',{job_id:randomUUID(),command:'true',services:[]})).error.code,12419,'enrollment refuses v1 downgrade');
 await delay(50);
 assert.equal((await call('authority_renew',{}, {...leased,expires_at_ms:Date.now()+45000})).accepted,true);
 const oldSocket=socket,oldRuntime=profile.runtime_id,blipAt=performance.now();
 offlineUntil=Date.now()+7000;socket=undefined;oldSocket.terminate();
 await waitFor(()=>socket&&socket!==oldSocket&&socket.readyState===1,'authority reconnect after seven-second blip',30000);
 assert.equal(profile.runtime_id,oldRuntime,'socket recovery must preserve the runtime');
 const afterBlip=await call('job',{job_id:leasedJob},authority());
 assert.equal(afterBlip.status,'running','a brief WS outage must not cancel a long job');
 assert.equal((await call('authority_renew',{}, {...leased,expires_at_ms:Date.now()+45000})).accepted,true);
 console.log('Authority disconnect survived ms:',(performance.now()-blipAt).toFixed(2));
 const revokeAt=performance.now();
 assert.equal((await call('authority_revoke',{},authority({revision:2}))).accepted,true);
 let cancelled;
 for(let n=0;n<200;n++){
  cancelled=await call('job',{job_id:leasedJob},authority({revision:2}));
  if(cancelled.status==='finished')break;
  await delay(25);
 }
 assert.equal(cancelled.status,'finished',JSON.stringify(cancelled));
 console.log('Authority revoke-to-finished ms:',(performance.now()-revokeAt).toFixed(2));
 assert.equal((await call('authority_renew',{}, {...leased,expires_at_ms:Date.now()+45000})).error.code,12421,'revoked lease cannot renew');
 assert.equal((await call('exec',{job_id:randomUUID(),command:'true',services:[]},authority())).error.code,12421,'old revision cannot restart');
 const expiring=authority({revision:2,expires_at_ms:Date.now()+5000}),expiryJob=randomUUID();
 const startedExpiry=await call('exec',{job_id:expiryJob,command:'sleep 60',background:true,services:[]},expiring);
 assert.equal(startedExpiry.job_id,expiryJob,JSON.stringify(startedExpiry));
 await delay(5200);
 let expired;
 for(let n=0;n<200;n++){
  expired=await call('job',{job_id:expiryJob},authority({revision:2}));
  if(expired.status==='finished')break;
  await delay(25);
 }
 assert.equal(expired.status,'finished','lack of renewal stops the background process');
 assert.equal((await call('exec',{job_id:randomUUID(),command:'true',services:[]},authority({revision:3}))).exit_code,0,'fresh revision recovers');
 console.log('Authority v2: capability denial, renewal, revocation, late renewal, expiry and v1 downgrade passed');
 await contexts({call,callWithReconnect,frames,profile,conversationId,turnId,origin,browserUrl,certificate:`${testDirectory}/tls.crt`});
 for(const line of output.join('').split('\n').filter(line=>line.includes('secure_browser_startup')))console.log(line);
 for(const secret of [token,auth,signing.toString('hex')])assert(!output.join('').includes(secret),'node logs must not contain credentials');
 assert(!output.join('').includes('stderr-secret-fixture'),'developer diagnostics must never expose child stderr');
 console.log('| Scenario | Changed frames/s | Frame bytes/s | Actions |\n|---|---:|---:|---:|');
 for(const sample of performanceSamples)console.log(`| ${sample.scenario} | ${sample.fps.toFixed(2)} | ${sample.bytes_per_second.toFixed(0)} | ${sample.actions} |`);
 console.log(JSON.stringify({passed:true,capabilities:profile,desktop_frames:frames.length,file_round_trip_mib:4,file_round_trip_ms:transferMs,takeover_ms:takeoverMs,renderer_sandbox:true,agent_no_new_privs:true,agent_namespace_filter:true,desktop_performance:performanceSamples,input_to_frame_ms:{p50:latencies[4],p95:latencies[9]}}));
} catch(error){
 console.error(error.message);
 console.error('Node diagnostics:',output.join('').slice(-15000).replaceAll(token,'[redacted]').replaceAll(auth,'[redacted]').replaceAll(signing.toString('hex'),'[redacted]'));
 if(profile)console.error('Capability status:',JSON.stringify(profile));
 process.exitCode=1;
}finally{
 await Promise.all(screenshotWrites);
 clearInterval(heartbeat);if(child?.pid){try{process.kill(-child.pid,'SIGTERM');}catch{}}
 for(const c of wss.clients)c.terminate();wss.close();server.close();website?.close();browserSite?.close();
 await delay(300);if(child?.pid){try{process.kill(-child.pid,'SIGKILL');}catch{}}
}
