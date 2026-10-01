// Run in the production machine image, with this directory and the `ws` package
// mounted read-only under /test. All credential values are generated in memory.
import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {createHash,createHmac,randomBytes,randomUUID} from 'node:crypto';
import {once} from 'node:events';
import http from 'node:http';
import https from 'node:https';
import fs from 'node:fs/promises';
import {createRequire} from 'node:module';
const {WebSocketServer}=createRequire(import.meta.url)('ws');
const signing=randomBytes(32), nodeId=randomUUID(), auth=`nyx_nauth_${randomBytes(32).toString('hex')}`;
const token=`nyx_nreg_${randomBytes(32).toString('hex')}`;
const responses=new Map(), transfers=new Map(), output=[], frames=[];
let socket, profile, child, website;
const values={username:`user-${randomBytes(12).toString('hex')}@example.test`,password:randomBytes(24).toString('base64url'),one_time_code:''};
const totpKey=randomBytes(20);
function totp(){const counter=Buffer.alloc(8);counter.writeBigUInt64BE(BigInt(Math.floor(Date.now()/30000)));const digest=createHmac('sha1',totpKey).update(counter).digest(),offset=digest[19]&15;return String((digest.readUInt32BE(offset)&0x7fffffff)%1000000).padStart(6,'0');}
const hash=value=>createHash('sha256').update(value).digest('hex');
const siteEvents=[],results=[];
function run(command,args){const result=spawnSync(command,args,{encoding:'utf8'});assert.equal(result.status,0,`${command} failed: ${result.stderr}`);return result.stdout;}
const server=http.createServer((_req,res)=>{res.writeHead(200,{'content-type':'application/json'});res.end('{"items":[]}');});
const wss=new WebSocketServer({server});
wss.on('connection',connection=>connection.on('message',(raw,binary)=>{
  if(binary){
    if(raw.subarray(0,4).toString()==='NYXM')frames.push({at:performance.now(),bytes:raw.length,kind:raw[4]});
    else {const stream=transfers.get(raw.subarray(0,36).toString());if(stream){stream.chunks.push(raw.subarray(36));stream.size+=raw.length-36;assert(stream.size<=5*1024*1024);}}
    return;
  }
  const message=JSON.parse(raw);
  if(message.type==='register'){
    assert.equal(message.token,token);
    connection.send(JSON.stringify({type:'register_ok',node_id:nodeId,auth_token:auth,signing_secret:signing.toString('hex')}));
  }else if(message.type==='auth'){
    socket=connection;
    connection.send(JSON.stringify({type:'auth_ok',heartbeat_interval_secs:10,capabilities:{proxy_binary_chunks:true}}));
  }else if(message.capabilities?.machine){profile=message.capabilities.machine;}
  else if(message.type==='proxy_response_start'){assert.equal(message.status,200);}
  else if(message.type==='proxy_response_end'){const stream=transfers.get(message.request_id);if(stream){stream.resolve(Buffer.concat(stream.chunks));transfers.delete(message.request_id);}}
  else if(message.type==='proxy_error'){transfers.get(message.request_id)?.reject(new Error('file transfer refused'));transfers.delete(message.request_id);}
  else if(message.type==='machine_service_call'&&message.operation==='job_finished'){connection.send(JSON.stringify({type:'machine_job_finished_ack',request_id:message.request_id}));}
  else if(message.type==='machine_result'){responses.get(message.request_id)?.(message.result);responses.delete(message.request_id);}
}));
server.listen(0,'127.0.0.1');await once(server,'listening');
const heartbeat=setInterval(()=>socket?.send(JSON.stringify({type:'heartbeat_ping'})),3000);
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function waitFor(check,label,ms=45000){const end=Date.now()+ms;while(Date.now()<end){if(check())return;await delay(5);}throw new Error(`Timed out: ${label}`);}
function canonical(value){if(Array.isArray(value))return `[${value.map(canonical).join(',')}]`;if(value && typeof value==='object')return `{${Object.keys(value).sort().map(k=>JSON.stringify(k)+':'+canonical(value[k])).join(',')}}`;return JSON.stringify(value);}
function request(operation,parameters){
 if(operation==='exec')parameters={...parameters,runtime_id:profile.runtime_id};
 const r={type:'machine_request',request_id:randomUUID(),node_id:nodeId,operation,parameters,timestamp:Math.floor(Date.now()/1000),nonce:randomUUID()};
 const mac=createHmac('sha256',signing).update(Buffer.from('nyxid.machine.request.v1\0'));
 const time=Buffer.alloc(8);time.writeBigInt64BE(BigInt(r.timestamp));
 for(const field of [r.request_id,nodeId,JSON.stringify(operation),createHash('sha256').update(canonical(parameters)).digest(),time,r.nonce]){
  const bytes=Buffer.isBuffer(field)?field:Buffer.from(field),length=Buffer.alloc(8);length.writeBigUInt64BE(BigInt(bytes.length));mac.update(length).update(bytes);
 }
 r.signature=mac.digest('hex');return r;
}
async function call(operation,parameters){
 const message=request(operation,parameters);
 const response=new Promise((resolve,reject)=>{const timeout=setTimeout(()=>{responses.delete(message.request_id);reject(new Error(`Timed out: ${operation}`));},40000);responses.set(message.request_id,result=>{clearTimeout(timeout);resolve(result);});});
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
  if(req.method==='POST'){let body='';req.on('data',chunk=>{body+=chunk;});req.on('end',()=>{const event=JSON.parse(body);if(req.url==='/signin'){const valid=event.username===values.username&&event.password===values.password&&event.one_time_code===totp();siteEvents.push({signed_in:valid});res.end(valid?'Signed in':'Invalid login');}else{siteEvents.push(event);res.end('ok');}});return;}
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
 child=spawn('/usr/local/bin/nyxid-machine-entrypoint',[],{env:{...process.env,NYXID_NODE_TOKEN:token,NYXID_NODE_URL:`ws://127.0.0.1:${server.address().port}/api/v1/nodes/ws`},detached:true,stdio:['ignore','pipe','pipe']});
 for(const stream of [child.stdout,child.stderr])stream.on('data',bytes=>{output.push(bytes.toString());});
 await waitFor(()=>profile,'machine capabilities',60000);
 assert.equal(profile.browser_isolated,true);
 assert.equal(profile.computer_ready,true,'cua MCP must be ready');
 assert.equal(profile.saved_login_ready,true,'production signed extension and native host must connect');
 // Production renderers must use nested user/PID namespaces and seccomp.
 const renderers=[];
 for(const pid of await fs.readdir('/proc')) {
  if(!/^\d+$/.test(pid))continue;
  const args=await fs.readFile(`/proc/${pid}/cmdline`).then(b=>b.toString().split(/[\0\s]+/),()=>[]);
  if(!args.includes('--type=renderer'))continue;
  assert(!args.includes('--no-sandbox'));
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
 assert.deepEqual(policy.URLBlocklist,['javascript:*']);assert.equal(policy.PasswordManagerEnabled,false);
 assert.equal(policy.ExtensionSettings[policy.ExtensionInstallForcelist[0].split(';')[0]].installation_mode,'force_installed');
 // Readiness requires the production extension's admin/non-disableable check.
 const debugProfile=`${testDirectory}/debug-refusal`;await fs.mkdir(debugProfile);await fs.chown(debugProfile,1001,1001);
 const debugProbe=spawn('runuser',['-u','browser','--','chromium','--headless=new','--disable-setuid-sandbox',`--user-data-dir=${debugProfile}`,'--remote-debugging-port=0','about:blank'],{detached:true,stdio:['ignore','ignore','pipe']});
 let debugMessages='';debugProbe.stderr.on('data',bytes=>{debugMessages+=bytes.toString();});
 try{await waitFor(()=>/remote debugging.*(disallowed|disabled)|DevTools.*(disallowed|disabled)/i.test(debugMessages),'managed policy rejects DevTools',15000);assert(!debugMessages.includes('DevTools listening'));assert.equal(await fs.access(`${debugProfile}/DevToolsActivePort`).then(()=>true,()=>false),false);}
 finally{try{process.kill(-debugProbe.pid,'SIGTERM');}catch{}}
 const boundary=await call('exec',{job_id:randomUUID(),conversation_id:randomUUID(),command:`python3 - <<'CHECK'
import os,socket,ctypes,errno,platform,subprocess
assert 'NoNewPrivs:\t1' in open('/proc/self/status').read()
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
 values.one_time_code=totp();
 for(const [field,value]of Object.entries(values)){
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
 const session=randomUUID();await call('desktop_open',{session_id:session});
 await waitFor(()=>frames.length>0,'desktop frame');
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
 assert(takeoverMs<=150,`owner takeover took ${takeoverMs} ms`);
 assert.equal((await slowComputer).error.code,12408,'late cua result discarded');
 assert.equal(await interrupted,true,'in-flight large file worker cancelled');
 for(const pid of agentDrivers){
  const deadline=performance.now()+1000;
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
 for(const sample of performanceSamples.filter(s=>s.scenario!=='idle'))assert(sample.fps>=15,`${sample.scenario}: ${sample.fps} fps below 15`);
 assert(performanceSamples[0].bytes_per_second<1024,'idle bandwidth should be near zero');
 assert(latencies[9]<=100,`input-to-frame p95 ${latencies[9]} ms exceeds 100 ms`);
 await call('desktop_control',{session_id:session,viewer_id:'test-owner',owner:false,revision:4});
 for(const secret of [token,auth,signing.toString('hex')])assert(!output.join('').includes(secret),'node logs must not contain credentials');
 console.log('| Scenario | Changed frames/s | Frame bytes/s | Actions |\n|---|---:|---:|---:|');
 for(const sample of performanceSamples)console.log(`| ${sample.scenario} | ${sample.fps.toFixed(2)} | ${sample.bytes_per_second.toFixed(0)} | ${sample.actions} |`);
 console.log(JSON.stringify({passed:true,capabilities:profile,desktop_frames:frames.length,file_round_trip_mib:4,file_round_trip_ms:transferMs,takeover_ms:takeoverMs,renderer_sandbox:true,agent_no_new_privs:true,agent_namespace_filter:true,desktop_performance:performanceSamples,input_to_frame_ms:{p50:latencies[4],p95:latencies[9]}}));
} catch(error){
 console.error(error.message);
 console.error('Node diagnostics:',output.join('').slice(-5000).replaceAll(token,'[redacted]').replaceAll(auth,'[redacted]').replaceAll(signing.toString('hex'),'[redacted]'));
 if(profile)console.error('Capability status:',JSON.stringify(profile));
 process.exitCode=1;
}finally{
 clearInterval(heartbeat);if(child?.pid){try{process.kill(-child.pid,'SIGTERM');}catch{}}
 for(const c of wss.clients)c.terminate();wss.close();server.close();website?.close();
 await delay(300);if(child?.pid){try{process.kill(-child.pid,'SIGKILL');}catch{}}
}
