import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {randomUUID} from 'node:crypto';
import {test} from 'node:test';
import vm from 'node:vm';

const source = name => readFileSync(new URL(`../resources/machine-browser/${name}.js`, import.meta.url), 'utf8');
const origin = 'https://signin.example';
const request = (field = 'password') => ({nonce: randomUUID(), field, allowed_origins: [origin], expires_at_ms: Date.now() + 20000});

function fixture() {
  let receive;
  const listeners = new Map(), observers = [];
  class Input {
    type = 'password'; isConnected = true; disabled = false; readOnly = false; value = '';
    getClientRects() { return [{}]; }
    select() {}
  }
  const input = new Input();
  const document = {
    activeElement: input, visibilityState: 'visible', hasFocus: () => true,
    addEventListener: (name, callback) => listeners.set(name, callback),
    execCommand: (_command, _ui, value) => { input.value = value; return true; },
  };
  const context = vm.createContext({
    URL, TextEncoder, Date, crypto: {randomUUID}, HTMLInputElement: Input,
    location: {origin}, document,
    window: {addEventListener() {}, removeEventListener() {}},
    MutationObserver: class { constructor(callback) { observers.push(callback); } observe() {} disconnect() {} },
    chrome: {runtime: {id: 'supervised', onMessage: {addListener(callback) { receive = callback; }}}},
  });
  vm.runInContext(source('policy') + '\n' + source('content'), context);
  return {
    input, document, context, listeners, observers,
    call(message, sender = 'supervised') {
      let response;
      receive(message, {id: sender}, value => { response = JSON.parse(JSON.stringify(value)); });
      return response;
    },
  };
}

test('policy requires exact HTTPS origins, bounded values and suitable input kinds', () => {
  const context = vm.createContext({URL, TextEncoder, Date});
  const policy = vm.runInContext(source('policy') + '\nNyxIdFillerPolicy', context);
  assert.equal(policy.validate(request()), null);
  for (const bad of ['http://signin.example', origin + '/', origin + '/login', 'https://user:pass@signin.example']) {
    assert.equal(policy.validate({...request(), allowed_origins: [bad]}), 'origin_mismatch');
  }
  assert.equal(policy.validate({...request(), expires_at_ms: Date.now() - 1}), 'expired');
  assert.equal(policy.validate({...request(), nonce: 'unbound'}), 'invalid_nonce');
  for (const value of ['', 'a\nb', 'a\0b', 'x'.repeat(16385)]) assert.equal(policy.valueAllowed(value), false);
  for (const type of ['text', 'email', 'tel', 'password', 'number', 'hidden', 'file', 'checkbox']) {
    assert.equal(policy.suitable('password', type), type === 'password');
    assert.equal(policy.suitable('username', type), ['text', 'email', 'tel'].includes(type));
    assert.equal(policy.suitable('one_time_code', type), ['text', 'number', 'tel'].includes(type));
  }
});

test('isolated content refuses foreign origins, hidden/wrong fields and foreign senders without typing', () => {
  const f = fixture(), base = request();
  assert.equal(f.call({...base, operation: 'probe'}, 'other-extension'), undefined);
  assert.equal(f.call({...base, allowed_origins: ['https://other.example']}).reason, 'origin_mismatch');
  assert.equal(f.call({...base, allowed_origins: ['https://*.example']}).reason, 'origin_mismatch');
  f.input.type = 'text';
  assert.equal(f.call({...base, operation: 'probe'}).reason, 'wrong_field');
  f.input.type = 'password'; f.document.visibilityState = 'hidden';
  assert.equal(f.call({...base, operation: 'probe'}).reason, 'not_focused');
  assert.equal(f.input.value, '');
});

test('one probed field consumes one nonce, checks focus again and pins passwords against reveal and copy', () => {
  const f = fixture(), base = request(), secret = randomUUID();
  const ready = f.call({...base, operation: 'probe'});
  assert.equal(ready.status, 'ready');
  const filled = f.call({...base, operation: 'fill', token: ready.token, value: secret});
  assert.deepEqual(filled, {status: 'filled', field: 'password', origin});
  assert.equal(f.input.value, secret);
  assert.equal(JSON.stringify(filled).includes(secret), false);
  assert.equal(f.call({...base, operation: 'fill', token: ready.token, value: secret}).reason, 'replayed');
  f.input.type = 'text'; f.observers[0]();
  assert.equal(f.input.type, 'password');
  for (const kind of ['copy', 'cut']) {
    let prevented = false;
    f.listeners.get(kind)({composedPath: () => [f.input], preventDefault() { prevented = true; }, stopImmediatePropagation() {}});
    assert.equal(prevented, true);
  }
  const next = request(), probe = f.call({...next, operation: 'probe'});
  f.document.activeElement = new f.input.constructor();
  assert.equal(f.call({...next, operation: 'fill', token: probe.token, value: secret}).reason, 'focus_changed');
  assert.equal(f.document.activeElement.value, '');
});

test('signed CRX is fresh and its package pin matches the deterministic sources', async () => {
  const {createHash, createPublicKey, verify} = await import('node:crypto');
  const {spawnSync} = await import('node:child_process');
  const {fileURLToPath} = await import('node:url');
  const root = fileURLToPath(new URL('../resources/machine-browser/', import.meta.url));
  const built = spawnSync('python3', ['-c', `import io,sys,zipfile,pathlib
out=io.BytesIO()
with zipfile.ZipFile(out,'w',zipfile.ZIP_DEFLATED) as z:
 for p in sorted(pathlib.Path(sys.argv[1]).glob('*.js'))+[pathlib.Path(sys.argv[1])/'manifest.json']:
  i=zipfile.ZipInfo(p.name,(2026,1,1,0,0,0));i.compress_type=zipfile.ZIP_DEFLATED;i.external_attr=0o100644<<16;z.writestr(i,p.read_bytes())
sys.stdout.buffer.write(out.getvalue())`, root]);
  assert.equal(built.status, 0);
  const crx = readFileSync(new URL('../resources/machine-browser/filler.crx', import.meta.url));
  assert.equal(crx.subarray(0,4).toString(), 'Cr24');
  assert.equal(crx.readUInt32LE(4), 3);
  const headerEnd = 12 + crx.readUInt32LE(8);
  assert.deepEqual(crx.subarray(headerEnd), built.stdout, 'Extension sources changed: run node cli/scripts/package-machine-filler.mjs');
  function fields(bytes) {
    let at=0;
    const variable=()=>{let value=0,shift=0,b;do{assert(at<bytes.length);b=bytes[at++];value+=(b&127)*2**shift;shift+=7;assert(shift<=35);}while(b&128);return value;};
    const result=new Map();
    while(at<bytes.length){const tag=variable();assert.equal(tag&7,2);const size=variable();assert(at+size<=bytes.length);result.set(tag>>>3,bytes.subarray(at,at+size));at+=size;}
    return result;
  }
  const header=fields(crx.subarray(12,headerEnd)), proof=fields(header.get(2)), signed=header.get(10000);
  const key=proof.get(1), digest=createHash('sha256').update(key).digest().subarray(0,16);
  assert.deepEqual(fields(signed).get(1),digest);
  const length=Buffer.alloc(4);length.writeUInt32LE(signed.length);
  assert(verify('sha256',Buffer.concat([Buffer.from('CRX3 SignedData\0'),length,signed,built.stdout]),createPublicKey({key,format:'der',type:'spki'}),proof.get(2)));
  const pin=JSON.parse(readFileSync(new URL('../resources/machine-browser/package.json',import.meta.url)));
  assert.equal(pin.sha256,createHash('sha256').update(crx).digest('hex'));
  assert.equal(pin.extension_id,digest.toString('hex').replace(/[0-9a-f]/g,c=>String.fromCharCode(97+parseInt(c,16))));
  assert.equal(pin.version,JSON.parse(readFileSync(new URL('../resources/machine-browser/manifest.json',import.meta.url))).version);
});

function browserFixture() {
  class Element {
    isConnected = true; disabled = false; readOnly = false; labels = [];
    tagName = 'INPUT'; autocomplete = ''; type = 'text'; id = randomUUID();
    get value() { throw new Error('input values must never be read'); }
    getClientRects() { return [{}]; }
    getBoundingClientRect() { return {left:10,top:10,right:110,bottom:40}; }
    scrollIntoView() {}
    contains(element) { return element === this; }
    closest() { return null; }
    getAttribute(name) { return name === 'aria-label' ? this.id : null; }
    matches(selector) { return selector.includes('input'); }
    focus() { document.activeElement = this; }
    select() {}
  }
  const rows = [new Element(), new Element(), new Element(), new Element()];
  rows[1].type = 'password'; rows[2].autocomplete = 'section-login current-password'; rows[3].autocomplete = 'one-time-code';
  let typed = 0;
  const document = {title: 'Fixture', readyState: 'complete', body: {}, activeElement: rows[0],
    querySelectorAll(selector) { return selector.includes('h1') ? [] : rows; },
    getElementById() {}, createTreeWalker() { return {nextNode() {return null;}}; },
    execCommand() { typed++; return true; },
  };
  document.body.querySelectorAll = document.querySelectorAll;
  document.elementFromPoint = () => document.activeElement;
  let painted = 0;
  const context = vm.createContext({document, crypto:{randomUUID}, HTMLInputElement:Element, HTMLSelectElement:class {},
    TextEncoder, NodeFilter:{SHOW_TEXT:4}, location:{href:origin}, getComputedStyle:()=>({}),
    setTimeout, clearTimeout, requestAnimationFrame:callback=>{painted++;queueMicrotask(callback);},
    innerWidth:1280,innerHeight:720,outerWidth:1280,outerHeight:800,devicePixelRatio:1,screenX:0,screenY:0,scrollX:0,scrollY:0,
    Event:class {}, window:{},
  });
  vm.runInContext(source('browser-actions'), context);
  return {api:context.NyxIdBrowser, rows, document, typed:()=>typed, painted:()=>painted};
}

test('browser refs remain stable, protected inputs cannot be typed or read, and snapshots are bounded', async () => {
  const f = browserFixture();
  const first = f.api.snapshot(), second = f.api.snapshot();
  assert.equal(first.elements[0].ref, second.elements[0].ref);
  assert(!JSON.stringify(first).includes('value'));
  for (const row of first.elements.slice(1)) {
    assert.equal(row.protected,true);
    await assert.rejects(f.api.act({action:'type',ref:row.ref,text:'forbidden'}), /protected_input/);
  }
  assert.equal(f.typed(),0);
  f.rows[0].isConnected=false;
  await assert.rejects(f.api.act({action:'click',ref:first.elements[0].ref}), /stale_ref/);
  await assert.rejects(f.api.act({action:'evaluate',expression:'document.cookie'}), /action_not_supported/);
  for(let n=0;n<100;n++) { const row=new f.rows[0].constructor(); row.id='long label '.repeat(50); f.rows.push(row); }
  assert(Buffer.byteLength(JSON.stringify(f.api.snapshot()))<=6000);
});

test('trusted keyboard preparation confirms focus and presentation without reading field values', async () => {
  const fixture=browserFixture();
  const ref=fixture.api.snapshot().elements[0].ref;
  fixture.document.activeElement=fixture.rows[1];
  const prepared=await fixture.api.act({action:'_prepare',input_action:'type',ref,text:'ordinary text'});
  assert.equal(prepared.focused,true);
  assert.equal(fixture.document.activeElement,fixture.rows[0]);
  assert.equal(fixture.painted(),2);
  assert.equal(fixture.typed(),0,'preflight does not dispatch input');
});

test('partially visible tall frames retain their visible intersection', () => {
  const fixture=browserFixture();
  const frame={clientLeft:0,clientTop:0,clientWidth:800,clientHeight:1600,offsetWidth:800,offsetHeight:1600,
    getBoundingClientRect:()=>({left:10,top:-1200,width:800,height:1600})};
  fixture.document.activeElement=frame;
  const point=fixture.api.framePoint(frame,{x:400,y:800,width:800,height:1600,
    visible_rect:{left:0,top:0,right:800,bottom:1600}});
  assert.equal(point.x,410);assert.equal(point.y,200);
  assert.equal(point.visible_rect.top,0);assert.equal(point.visible_rect.bottom,400);
});

test('browser navigation waits for a loaded document and never replays a click whose response is lost', async () => {
  let clicks=0, snapshots=0, polls=0;
  const tab={id:12,title:'Fixture',url:origin,active:true};
  const chrome={webNavigation:{getAllFrames:async()=>[{frameId:0,parentFrameId:-1}]},tabs:{
    query:async()=>[tab], get:async()=>({...tab,status:++polls===1?'loading':'complete'}),
    update:async()=>tab,
    sendMessage:async(_id,request)=>{
      if(request.action==='click'){clicks++;throw new Error('document replaced');}
      snapshots++;return {status:'ok',snapshot:{text:'new document',ready:'complete'}};
    },
  }};
  const context=vm.createContext({chrome,URL,TextEncoder,Date,setTimeout:callback=>callback()});
  const api=vm.runInContext(source('browser-background')+'\nNyxIdBrowserBackground',context);
  const result=await api.perform({action:'click',ref:'old'});
  assert.equal(clicks,1);assert.equal(snapshots,1);assert.equal(polls,2);
  assert.equal(result.status,'ok');assert.equal(result.snapshot.tab_id,'12');
  assert.equal(result.snapshot.tabs[0].id,'12');
  await assert.rejects(api.perform({action:'navigate',url:'file:///etc/passwd'}),/invalid_url/);
});

test('large pages paginate every element and queries narrow labels without reading values', () => {
  const f=browserFixture();const Input=f.rows[0].constructor;f.rows.splice(0);
  for(let n=0;n<500;n++){const row=new Input();row.id=`Row ${String(n).padStart(3,'0')}`;f.rows.push(row);}
  let offset=0;const refs=[];
  do {
    const page=f.api.snapshot({offset});
    assert(Buffer.byteLength(JSON.stringify(page))<=6000);
    assert(page.elements.length>0);refs.push(...page.elements.map(e=>e.ref));
    if(!page.more.elements)break;
    assert(page.next_offset>offset);offset=page.next_offset;
  } while(offset<500);
  assert.equal(refs.length,500);assert.equal(new Set(refs).size,500);
  const filtered=f.api.snapshot({query:{role:'textbox',label:'Row 499'}});
  assert.equal(filtered.elements.length,1);assert.equal(filtered.elements[0].label,'Row 499');
});

test('visible frames aggregate with prefixed refs, route actions, and preserve pagination bounds', async () => {
  const tab={id:12,title:'Frames',url:origin,active:true,status:'complete'};
  const calls=[];
  const chrome={windows:{update:async()=>{}},webNavigation:{getAllFrames:async()=>Array.from({length:12},(_,i)=>({frameId:i,parentFrameId:i?0:-1}))},tabs:{
    query:async()=>[tab],get:async()=>tab,update:async()=>tab,
    sendMessage:async(_tab,request,{frameId})=>{
      calls.push({request,frameId});
      if(request.action==='_visibility')return {visible:true,point:{x:10,y:10,width:100,height:100}};
      if(request.action==='_frame_parent')return {status:'ok',point:request.point};
      if(request.action==='_frame_signal'||request.action==='_frame_assert')return {status:'ok'};
      if(request.action==='click')return {status:'ok'};
      if(request.action==='type')return {status:'refused',reason:'protected_input'};
      const count=100,offset=request.offset||0;
      return {snapshot:{url:origin+'/frame'+frameId,title:'Frame',ready:'complete',headings:['Heading'],text:'visible '.repeat(190),total:count,
        more:{elements:offset+60<count,text:true},elements:Array.from({length:Math.max(0,Math.min(60,count-offset))},(_,i)=>({ref:`item${offset+i}`,label:`Frame ${frameId} row ${offset+i}`}))}};
    },
  }};
  const context=vm.createContext({chrome,URL,TextEncoder,Date,crypto:{randomUUID},setTimeout:callback=>callback()});
  const api=vm.runInContext(source('browser-background')+'\nNyxIdBrowserBackground',context);
  const first=await api.perform({action:'snapshot'});
  assert(first.snapshot.more.elements);assert(first.snapshot.more.text);
  assert(first.snapshot.frames.length>1);assert(first.snapshot.elements.every(e=>e.ref.startsWith('f0:')),'do not skip a truncated frame tail');
  assert(Buffer.byteLength(JSON.stringify(first))<=9000);
  const next=await api.perform({action:'snapshot',offset:100});
  assert(next.snapshot.elements[0].ref.startsWith('f1:'));
  const scoped=await api.perform({action:'snapshot',scope:'f3:item1'});
  assert(scoped.snapshot.elements.every(e=>e.ref.startsWith('f3:')));
  await api.perform({action:'click',ref:'f7:item2'});
  assert(calls.some(c=>c.frameId===7&&c.request.action==='click'&&c.request.ref==='item2'));
  const protected_=await api.perform({action:'type',ref:'f7:secret',text:'never'});
  assert.equal(protected_.reason,'protected_input');
});

test('a spoofed frame rendezvous cannot redirect trusted input into a sibling', async () => {
  const tab={id:1,url:origin,windowId:1};const seen=[];
  const chrome={windows:{update:async()=>{}},webNavigation:{getAllFrames:async()=>[{frameId:0,parentFrameId:-1},{frameId:7,parentFrameId:0}]},tabs:{
    query:async()=>[tab],update:async()=>tab,
    sendMessage:async(_tab,request,{frameId})=>{
      seen.push([request.action,frameId]);
      if(request.action==='_prepare')return {status:'ok',snapshot:{point:{x:10,y:10,width:100,height:100}}};
      if(request.action==='_frame_parent')return {status:'ok',index:3,point:request.point};
      if(request.action==='_frame_assert'){assert.equal(frameId,7);assert.equal(request.index,3);return {status:'refused'};}
      return {status:'ok'};
    },
  }};
  const context=vm.createContext({chrome,URL,TextEncoder,Date,crypto:{randomUUID},setTimeout:callback=>callback()});
  const api=vm.runInContext(source('browser-background')+'\nNyxIdBrowserBackground',context);
  const result=await api.perform({action:'_prepare',input_action:'click',ref:'f7:button'});
  assert.equal(result.status,'refused');assert.equal(result.reason,'overlay_mismatch');
  assert(seen.some(([action])=>action==='_frame_assert'));
  assert(!seen.some(([action])=>action==='click'));
});


function nativeFixture() {
  const events = {}, timers = [], ports = [];
  let failures = 0;
  const event = name => ({addListener(callback) { events[name] = callback; }});
  const context = vm.createContext({
    importScripts() {}, Date, Map,
    setTimeout(callback, ms) { timers.push({callback,ms}); return timers.length; },
    chrome: {
      management: {async getSelf() { if(failures-- > 0)throw new Error('temporary failure');return {installType:'admin',mayDisable:false}; }},
      runtime: {
        id:'pinned', onStartup:event('startup'), onInstalled:event('installed'),
        connectNative(name) {
          assert.equal(name,'dev.nyxid.machine_filler');
          const sent=[];
          const port = {sent,onDisconnect:event(`disconnect${ports.length}`),onMessage:event(`message${ports.length}`),postMessage(message) { sent.push(message); }};
          ports.push(port);return port;
        },
      },
    },
  });
  vm.runInContext(source('background'),context);
  return {events,timers,ports,failNext(count) { failures=count; }};
}
const flushNative = () => new Promise(setImmediate);
const incomingNative = () => ({operation:'browser',nonce:randomUUID(),expires_at_ms:Date.now()-1});

test('MV3 wake listeners keep one native port and inbound messages reset reconnect backoff', async () => {
  const {events,timers,ports}=nativeFixture();
  assert.equal(typeof events.startup,'function','listener registration must be synchronous');
  assert.equal(typeof events.installed,'function');
  events.startup();events.installed();await flushNative();
  assert.equal(ports.length,1,'top-level and startup/install must not duplicate connections');
  for(let i=0;i<8;i++){
    await events[`message${i}`](incomingNative());
    events[`disconnect${i}`]();assert.equal(timers.length,1);
    const timer=timers.shift();assert.equal(timer.ms,500);
    timer.callback();await flushNative();assert.equal(ports.length,i+2);
  }
  events.startup();await flushNative();assert.equal(ports.length,9);
});

test('asynchronous native-host failures after hello back off to 4 seconds until an inbound message', async () => {
  const f=nativeFixture(), {events,timers,ports}=f;
  await flushNative();
  // connectNative and postMessage both succeed; the actual connection failure
  // arrives later through onDisconnect, as with an unreachable filler.sock.
  for(const [i,ms] of [500,1000,2000,4000,4000,4000].entries()){
    assert.equal(ports[i].sent[0].type,'hello');
    events[`disconnect${i}`]();assert.equal(timers.length,1);
    const timer=timers.shift();assert.equal(timer.ms,ms);
    timer.callback();await flushNative();assert.equal(ports.length,i+2);
  }
  await events.message6(incomingNative());
  events.disconnect6();assert.equal(timers[0].ms,500,'a proven connection resets the backoff');
  // Synchronous failures also keep backing off; neither failure path resets it.
  f.failNext(6);
  for(const ms of [500,1000,2000,4000,4000,4000,4000]){
    const timer=timers.shift();assert.equal(timer.ms,ms);
    timer.callback();await flushNative();
  }
  events.disconnect7();assert.equal(timers.shift().ms,4000,'hello alone never resets the backoff');
});
