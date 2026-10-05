// Imported by the real container suite; requests use signed production authority.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';

export async function contexts({call,callWithReconnect,profile,conversationId,turnId,origin,browserUrl,certificate,frames}) {
 assert(profile.separated,'node must explicitly report separated support');
 const base={require_v2:true,generation:1,mode:'separated',owner_id:randomUUID(),actor_id:randomUUID(),group_id:null,runtime_id:profile.runtime_id,conversation_id:conversationId,turn_id:turnId,revision:1,capabilities:{shell:true,files:true,browser:true,computer:true,developer_browser:true}};
 const a={...base,context_id:randomUUID(),agent_id:randomUUID()},b={...base,context_id:randomUUID(),agent_id:randomUUID()};
 const auth=(context,extra={})=>({...context,lease_id:randomUUID(),expires_at_ms:Date.now()+45000,...extra});
 const invoke=async(context,operation,args)=>{
  const lease=auth(context);
  // Production renews active v2 authority every 10 s. A bounded 45 s
  // cold start plus one repair must not run with the fixture's one-shot lease.
  if(!['browser','computer','fill_login'].includes(operation))return call(operation,args,lease);
  let renewal=Promise.resolve();
  const timer=setInterval(()=>{renewal=renewal.then(()=>call('authority_renew',{}, {...lease,expires_at_ms:Date.now()+45000}));},10000);
  try{return await call(operation,args,lease,undefined,135000);}
  finally{clearInterval(timer);await renewal;}
 };
 const command=(context,command)=>invoke(context,'exec',{job_id:randomUUID(),command,services:[]});
 if(!profile.separated.available) {
  assert.equal(profile.separated.reason,'separated_requires_landlock_abi_6',JSON.stringify(profile.separated));
  const refused=await command(a,'printf must-not-run');
  assert.equal(refused.error?.code,12419,'unsupported hosts never fall back to shared execution');
  console.log('SKIP (unsupported host): separated requires live Landlock ABI 6; refusal verified');
  return;
 }

 for(const context of [a,b]) {
  const result=await command(context,'printf context-ok; pwd; id -u');
  assert.equal(result.exit_code,0,JSON.stringify(result));assert.match(result.stdout,/context-ok/);
  const writeAuthority=auth(context),writeRequestId=randomUUID(),writeParameters={path:'private.txt',content:context.context_id};
  const written=await (context===a?callWithReconnect:call)('write_file',writeParameters,writeAuthority,writeRequestId);
  if(written.error) {
   const diagnostic=await command(context,'pwd; id -u; ls -la .');
   console.error('Context workspace metadata on failure:',JSON.stringify(diagnostic));
  }
  assert(!written.error,`context workspace write: ${JSON.stringify(written)}`);
  // A reconnect can lose the response after the node has committed the
  // create-only write. Re-delivery keeps the original request id and must
  // return its receipt instead of executing the write a second time.
  const replay=await call('write_file',writeParameters,writeAuthority,writeRequestId);
  assert.deepEqual(replay,written,'replayed context write returns the original receipt');
}
 for(const context of [a,b]) {
  const read=await invoke(context,'read_file',{path:'private.txt'});
  assert.equal(read.content,context.context_id,'file worker reads its own workspace bytes');
  const denied=await command(context,'test ! -r /workspace; test ! -x /home/agent');
  assert.equal(denied.exit_code,0,'legacy workspace and home have DAC gates');
 }
 const pool=await command(a,`python3 -c 'from multiprocessing import Pool; p=Pool(2); assert p.map(abs,[-1,-2])==[1,2]; p.close(); p.join()'`);
 assert.equal(pool.exit_code,0,JSON.stringify(pool));
 const store=JSON.parse(await fs.readFile('/var/lib/nyxid-machine/node/machine-contexts.json','utf8'));
 const allocation=c=>store.contexts[c.context_id],role=(c,r)=>`/var/lib/nyxid-machine/contexts/${allocation(c).command_uid}/b${c.generation}/${r==='secure'?'s':'d'}`;
 const accounts=[allocation(a),allocation(b)].flatMap(r=>[r.command_uid,r.browsers['1'].secure_uid,r.browsers['1'].dev_uid]);
 assert.equal(new Set(accounts).size,6,'context roles have distinct UIDs');
 const run=(bin,args)=>{const result=spawnSync(bin,args,{encoding:'utf8'});assert.equal(result.status,0,`${bin}: ${result.stderr}`);return result.stdout;};
 // Replay the shipped entrypoint's filesystem initialization without launching
 // a second daemon/display into the live fixture. A restart must retain the
 // opted-in gate before any separated request arrives.
 const entrypoint=await fs.readFile('/usr/local/bin/nyxid-machine-entrypoint','utf8');
 const boundary=entrypoint.indexOf('# Each browser has an independent display');
 assert(boundary>0,'entrypoint filesystem initialization boundary');
 run('sh',['-c',entrypoint.slice(0,boundary)]);
 const legacyGate=await fs.stat('/workspace');
 assert.equal(legacyGate.uid,0,'startup preserves supervisor ownership after opt-in');
 assert.equal(legacyGate.mode&0o777,0o770,'startup preserves the legacy DAC gate');
 for(const c of [a,b]) {
  const uid=allocation(c).browsers['1'].secure_uid,name=`nyxc${uid}`,home=`${role(c,'secure')}/home`;
  run('runuser',['-u',name,'--','mkdir','-p',`${home}/.pki/nssdb`]);
  run('runuser',['-u',name,'--','certutil','-N','-d',`sql:${home}/.pki/nssdb`,'--empty-password']);
  run('runuser',['-u',name,'--','certutil','-A','-d',`sql:${home}/.pki/nssdb`,'-n','Fixture','-t','C,,','-i',certificate]);
 }
 const navigate=(c,url,browser='secure')=>invoke(c,'browser',{browser,action:'navigate',url});
 // Deterministic slow process start, without a production fault-injection
 // hook or altered signed extension. The wrapper keeps Chromium's PID. Both
 // first launches exceed the former 12 + 4 s grace. Optionally exercise the
 // complete 45 s failure and one fresh repair budget as well.
 const chromium='/usr/bin/chromium', real=chromium+'-cold-test-real';
 await fs.rename(chromium,real);
 const repair=process.env.NYXID_TEST_CONTEXT_COLD_REPAIR==='1';
 await fs.writeFile(chromium,`#!/usr/bin/python3
import os,sys,time
profile=next((a.split('=',1)[1] for a in sys.argv[1:] if a.startswith('--user-data-dir=')), '')
if '/contexts/' in profile:
 marker=os.path.join(profile,'.cold-start-test')
 try:
  with open(marker) as f: attempt=int(f.read())
 except FileNotFoundError: attempt=0
 with open(marker,'w') as f: f.write(str(attempt+1))
 if attempt==0: time.sleep(60 if ${repair?'True':'False'} and os.getuid()==${allocation(a).browsers['1'].secure_uid} else 18)
 elif attempt==1 and ${repair?'True':'False'} and os.getuid()==${allocation(a).browsers['1'].secure_uid}: time.sleep(18)
os.execv(${JSON.stringify(real)},[${JSON.stringify(real)}]+sys.argv[1:])
`,{mode:0o755});
 let navigated;
 try {
  navigated=await Promise.all([navigate(a,origin),navigate(b,browserUrl)]);
  for(const context of [a,b]) {
   const attempts=Number(await fs.readFile(`${role(context,'secure')}/browser-profile/.cold-start-test`,'utf8'));
   assert.equal(attempts,repair&&context===a?2:1,'cold-start budget preserves slow Chromium; repair has a fresh budget');
  }
 } finally {await fs.rename(real,chromium);}
 console.log('Separated cold start: delayed launch'+(repair?' and exhausted-budget repair':' without relaunch')+' passed');
 for(const result of navigated)assert(!result.error,`context secure navigation: ${JSON.stringify(result.error)}`);
 const username=navigated[0].snapshot?.elements?.find(e=>e.kind==='email');
 assert(username,'context A sign-in field is visible');
 const focused=await invoke(a,'browser',{action:'click',ref:username.ref});
 assert(!focused.error,`context secure focus: ${JSON.stringify(focused.error)}`);
 const fill=await invoke(a,'fill_login',{field:'username',allowed_origins:[origin],value:'context-a@example.test'});
 assert.equal(fill.status,'filled',JSON.stringify(fill));
 const mismatch=await invoke(b,'fill_login',{field:'username',allowed_origins:[origin],value:'never-cross-context'});
 assert.notEqual(mismatch.status,'filled','context B cannot fill context A focused page');
 for(const c of [a,b]) {
  const dev=await navigate(c,browserUrl,'dev');assert(!dev.error,JSON.stringify(dev));
  const evaluated=await invoke(c,'browser',{browser:'dev',action:'evaluate',expression:'document.title'});assert(!evaluated.error,JSON.stringify(evaluated));
  const screenshot=await invoke(c,'browser',{browser:'dev',action:'screenshot'});assert(!screenshot.error,JSON.stringify(screenshot));
 }
 let ax,lastWindows;
 const deadline=Date.now()+15000;
 while(Date.now()<deadline) {
  const windows=await invoke(b,'computer',{tool:'list_windows',arguments:{}});
  lastWindows=windows;
  assert(!windows.error&&!windows.isError,JSON.stringify(windows));
  assert(!JSON.stringify(windows).includes('NyxID sign-in test'),'computer sees only its context display');
  const window=windows.structuredContent?.windows?.find(w=>JSON.stringify(w).includes('Machine browser fixture'));
  if(window) ax=await invoke(b,'computer',{tool:'get_window_state',arguments:{pid:window.pid,window_id:window.window_id,include_screenshot:false,timeout_ms:5000}});
  if(ax&&!ax.isError&&JSON.stringify(ax).includes('Project catalog'))break;
  await new Promise(resolve=>setTimeout(resolve,100));
 }
 assert(ax&&!ax.error&&!ax.isError&&JSON.stringify(ax).includes('Project catalog'),`context D-Bus exposes its own browser accessibility tree: ${JSON.stringify({ax,windows:lastWindows}).slice(0,8000)}`);
 for(const [from,to] of [[a,b],[b,a]]) {
  const root=role(to,'secure'),uid=allocation(from).browsers['1'].secure_uid;
  run('runuser',['-u',`nyxc${uid}`,'--','python3','-c',`import os,socket
for path in [${JSON.stringify(root+'/browser-profile')},${JSON.stringify(root+'/home')},${JSON.stringify(root+'/r/filler.sock')}]:
 try: os.listdir(path) if os.path.isdir(path) else open(path).read()
 except (PermissionError,FileNotFoundError): pass
 else: raise AssertionError('cross-context profile readable')
s=socket.socket(socket.AF_UNIX)
try: s.connect(${JSON.stringify(root+'/r/filler.sock')})
except PermissionError: pass
else: raise AssertionError('cross-context native socket accessible')`]);
  for(const actor of [allocation(from).command_uid,allocation(from).browsers['1'].secure_uid,allocation(from).browsers['1'].dev_uid]) {
   const denied=[`${root}/browser-profile`,`${role(to,'dev')}/home`,`/var/lib/nyxid-machine/contexts/${allocation(to).command_uid}/command/workspace`,'/var/lib/nyxid-machine/node/config.toml','/var/lib/nyxid-machine/desktop/browser-profile','/workspace','/home/agent','/home/browser','/home/devbrowser'];
   const policyGroup=actor===allocation(from).command_uid?undefined:run('id',['-g',actor===allocation(from).browsers['1'].secure_uid?'browser':'devbrowser']).trim();
   run('setpriv',[`--reuid=${actor}`,`--regid=${actor}`,...(policyGroup?[`--groups=${policyGroup}`]:['--clear-groups']),'python3','-c',`import os
for path in ${JSON.stringify(denied)}:
 assert not os.access(path,os.R_OK), 'cross-context or legacy read permitted'
 assert not os.access(path,os.W_OK), 'cross-context or legacy write permitted'`]);
  }
  for(const targetRole of ['secure','dev']) {
   const targetUid=allocation(to).browsers['1'][`${targetRole}_uid`];
   const sockets=await fs.readdir('/tmp/.X11-unix');let display;
   for(const name of sockets)if((await fs.stat('/tmp/.X11-unix/'+name)).uid===targetUid)display=':'+name.slice(1);
   assert(display,'target context display socket');
   for(const sourceRole of ['secure','dev']) {
    const sourceUid=allocation(from).browsers['1'][`${sourceRole}_uid`];
    const denied=spawnSync('runuser',['-u',`nyxc${sourceUid}`,'--','env',`DISPLAY=${display}`,`XAUTHORITY=${role(from,sourceRole)}/home/.nyxid-dev-display/Xauthority`,'xdpyinfo'],{encoding:'utf8'});
    assert.notEqual(denied.status,0,'context cookie cannot authenticate another display');
   }
  }
  const attempt=await command(from,`cat ${role(to,'secure')}/browser-profile/Local\\ State`);
  assert.notEqual(attempt.exit_code,0,'command cannot read sibling browser');
 }
 const desktops=[];
 for(const context of [a,b])for(const display of ['secure','dev']) {
  const session_id=randomUUID(),parameters={context_id:context.context_id,display,session_id};
  const opened=await call('desktop_open',parameters);
  assert.equal(opened.streaming,true,JSON.stringify(opened));desktops.push(parameters);
 }
 const streamDeadline=Date.now()+15000;
 while(!desktops.every(d=>frames.some(f=>f.kind===1&&f.session===d.session_id.replaceAll('-','')))&&Date.now()<streamDeadline)await new Promise(resolve=>setTimeout(resolve,25));
 assert(desktops.every(d=>frames.some(f=>f.kind===1&&f.session===d.session_id.replaceAll('-',''))),'all four context displays stream independently');
 const controlled=desktops[0];
 const activeJobs=[];
 for(const context of [a,b]) {
  const job_id=randomUUID();
  const started=await invoke(context,'exec',{job_id,command:'sleep 30',background:true,services:[]});
  assert.equal(started.job_id,job_id,JSON.stringify(started));activeJobs.push(job_id);
 }
 assert.equal((await call('desktop_control',{...controlled,viewer_id:'context-owner',owner:true,revision:1})).error,undefined);
 const surviving=await invoke(b,'job',{job_id:activeJobs[1]});
 assert.equal(surviving.status,'running','takeover preserves the sibling running job');
 assert.equal((await invoke(b,'job_cancel',{job_id:activeJobs[1]})).cancel_requested,true);
 assert.equal((await command(a,'printf must-not-run')).error?.code,12408,'owner control blocks its context shell');
 assert.equal((await command(b,'true')).exit_code,0,'owner control does not block the sibling context');
 assert.equal((await call('desktop_control',{...controlled,viewer_id:'context-owner',owner:false,revision:2})).error,undefined);
 const cancelled=await invoke(a,'job',{job_id:activeJobs[0],wait_secs:5});
 assert.equal(cancelled.status,'finished','takeover cancels its running job');
 for(const desktop of desktops)assert.equal((await call('desktop_close',desktop)).error,undefined);
 assert.equal((await call('authority_revoke',{quarantine_profiles:true},auth(a,{revision:2}))).accepted,true);
 const stale=await invoke(a,'browser',{action:'snapshot'});assert(stale.error,'old generation refused after quarantine');
 a.generation=2;a.revision=2;
 const fresh=await navigate(a,browserUrl);assert(!fresh.error,JSON.stringify(fresh));
 const recovered=JSON.parse(await fs.readFile('/var/lib/nyxid-machine/node/machine-contexts.json','utf8')).contexts[a.context_id];
 assert(recovered.browsers['2'].secure_uid>recovered.browsers['1'].dev_uid,'quarantine creates new browser identities');
 assert.equal((await invoke(b,'browser',{action:'snapshot'})).error,undefined,'other context survives revocation');
 console.log('Separated contexts: two workspaces, secure/dev browsers, cross-profile/display/socket denial, four display streams, context takeover, login binding and profile quarantine passed');
}
