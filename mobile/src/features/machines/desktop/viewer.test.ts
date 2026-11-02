import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createDesktopViewer, refusalLabel, terminalDesktopFailure, type ViewerState, type ViewSocket } from './viewer';
import { desktopSocketUrl } from './desktopRoutes';
import { SESSION } from './testVectors';

function fixture() {
  let now=0, timerId=0, current=true, clears=0, invalidations=0;
  const timers=new Map<number,{at:number;run:()=>void}>();
  const sockets: (ViewSocket & { sent:(string | ArrayBuffer)[]; closed:boolean })[]=[];
  const states: ViewerState[]=[];
  const clock={now:()=>now,random:()=>0.5,later:(run:()=>void,ms:number)=>{ const id=++timerId;timers.set(id,{at:now+ms,run});return id; },cancel:(id:unknown)=>{timers.delete(id as number);}};
  const viewer=createDesktopViewer({ current:()=>current,clock,state:s=>states.push(s),frame:()=>{},clear:()=>{clears++;},invalidated:()=>{invalidations++;},connect:async()=>{
    const ws: ViewSocket & { sent:(string | ArrayBuffer)[]; closed:boolean }={readyState:1,binaryType:'',onopen:null,onmessage:null,onclose:null,onerror:null,
      sent:[],closed:false,send(value){this.sent.push(value);},close(){this.closed=true;}};
    sockets.push(ws);return ws;
  }});
  const advance=async(ms:number)=>{
    const end=now+ms;
    while(true) {const next=[...timers].filter(([,t])=>t.at<=end).sort((a,b)=>a[1].at-b[1].at)[0];if(!next)break;
      now=next[1].at;timers.delete(next[0]);next[1].run();await Promise.resolve();}
    now=end;await Promise.resolve();
  };
  const connected=()=>sockets.at(-1)?.onmessage?.({data:JSON.stringify({type:'connected',session_id:SESSION,controller:'agent'})});
  return {viewer,sockets,states,timers,advance,connected,clearCount:()=>clears,invalidations:()=>invalidations,revoke:()=>{current=false;}};
}
test('before explicit control only refresh_frame is sent, at most once per second', async()=>{
  const f=fixture();f.viewer.start();await f.advance(0);f.connected();
  for(let i=0;i<20;i++)f.viewer.refresh();
  assert.deepEqual(f.sockets[0]?.sent,['{"type":"refresh_frame"}']);
  await f.advance(999);f.viewer.refresh();assert.equal(f.sockets[0]?.sent.length,1);
  await f.advance(1);f.viewer.refresh();assert.equal(f.sockets[0]?.sent.length,2);
  f.viewer.stop();assert.equal(f.sockets[0]?.closed,true);assert.equal(f.timers.size,0);
  assert.equal(f.sockets[0]?.sent.every(v=>typeof v==='string'&&JSON.parse(v).type==='refresh_frame'),true);
});
test('idle pixels stay live with state heartbeats; missing state hits deadline', async()=>{
  const f=fixture();f.viewer.start();await f.advance(0);f.connected();
  for(let i=0;i<6;i++){await f.advance(10_000);f.sockets.at(-1)?.onmessage?.({data:'{"type":"state","controller":"owner"}'});}
  assert.equal(f.sockets.length,1);assert.equal(f.states.at(-1)?.phase,'attention');
  await f.advance(30_000);assert.equal(f.sockets[0]?.closed,true);assert.equal(f.states.at(-1)?.phase,'reconnecting');
  f.viewer.stop();
});
test('account/lifecycle invalidation clears pixels, pending timers and all socket callbacks',async()=>{
  const f=fixture();f.viewer.start();await f.advance(0);f.connected();f.revoke();await f.advance(100);
  assert.equal(f.sockets[0]?.closed,true);assert.equal(f.sockets[0]?.onmessage,null);assert.equal(f.timers.size,0);assert.ok(f.clearCount()>0);
  f.viewer.refresh();assert.equal(f.sockets[0]?.sent.length,0);
  assert.equal(f.invalidations(),1);f.viewer.refresh();assert.equal(f.invalidations(),1);
});
test('terminal policy/unsupported/unknown handshakes never retry; session transient reconnects are bounded',async()=>{
  for(const code of [1000,1002,1003,1008,1010]) {
    const f=fixture();f.viewer.start();await f.advance(0);f.connected();f.sockets[0]?.onclose?.({code});await f.advance(60_000);
    assert.equal(f.sockets.length,1);assert.equal(f.states.at(-1)?.phase,'unavailable');f.viewer.stop();
  }
  const refused=fixture();refused.viewer.start();await refused.advance(0);refused.sockets[0]?.onerror?.();await refused.advance(60_000);
  assert.equal(refused.sockets.length,1);refused.viewer.stop();
  const f=fixture();f.viewer.start();await f.advance(0);
  for(let i=0;i<5;i++){f.connected();f.sockets.at(-1)?.onclose?.({code:1006});await f.advance(10_000);}
  assert.equal(f.sockets.length,5);assert.equal(f.states.at(-1)?.phase,'unavailable');assert.equal(f.timers.size,0);f.viewer.stop();
  assert.equal(terminalDesktopFailure({statusCode:403}),true);assert.equal(terminalDesktopFailure({errorCode:12419}),true);
  assert.equal(terminalDesktopFailure({statusCode:503,errorCode:8001}),false);
});
test('one viewer survives and socket URL retains every selector',async()=>{
  const first=fixture();first.viewer.start();await first.advance(0);first.connected();
  const second=fixture();assert.equal(first.sockets[0]?.closed,true);second.viewer.stop();
  const params={nodeId:'n/a',conversationId:'c+ &',contextId:'ctx/+',display:'dev' as const};
  assert.equal(desktopSocketUrl('https://api.example/api/v1/mobile',params),'wss://api.example/api/v1/assistant/nyxagent/machines/n%2Fa/desktop?conversation_id=c%2B+%26&context_id=ctx%2F%2B&display=dev');
});

const click = {tool:'click',arguments:{x:12,y:8,button:'left',count:1}};
const ack = (f: ReturnType<typeof fixture>, controls = true) => f.sockets.at(-1)?.onmessage?.({data:JSON.stringify({type:'state',controller:controls?'owner':'agent',controls})});
test('input requires explicit human takeover AND server control acknowledgement',async()=>{
  const f=fixture();f.viewer.start();await f.advance(0);f.connected();
  f.viewer.input(click);f.viewer.stopTask();ack(f);
  f.viewer.input(click);assert.equal(f.sockets[0]?.sent.length,0,'server state alone cannot grant local intent');
  f.viewer.takeControl();f.viewer.input(click);assert.deepEqual(f.sockets[0]?.sent,['{"type":"take_control"}']);
  ack(f);f.viewer.input(click);
  const frame: unknown=f.sockets[0]!.sent.at(1);assert.ok(frame instanceof ArrayBuffer);
  const bytes=new Uint8Array(frame);assert.deepEqual([...bytes.slice(0,6)],[78,89,88,77,2,0]);
  assert.equal(new DataView(frame).getBigUint64(22),1n);
  assert.deepEqual(JSON.parse(new TextDecoder().decode(bytes.slice(30))),click);
  f.viewer.handBack();f.viewer.input(click);
  assert.equal(f.sockets[0]?.sent.length,3);assert.equal(f.sockets[0]?.sent[2],'{"type":"hand_back","note":""}');f.viewer.stop();
});
test('close/background/auth teardown returns only explicitly acquired control; never stops task',async()=>{
  for(const acquired of [false,true]){
    const f=fixture();f.viewer.start();await f.advance(0);f.connected();
    if(acquired){f.viewer.takeControl();ack(f);}
    f.viewer.stop();
    assert.deepEqual(f.sockets[0]?.sent,acquired?['{"type":"take_control"}','{"type":"hand_back","note":""}']:[]);
    assert.equal(f.sockets[0]?.closed,true);
  }
  const f=fixture();f.viewer.start();await f.advance(0);f.connected();f.viewer.takeControl();ack(f);f.revoke();await f.advance(100);
  assert.equal(f.sockets[0]?.sent.at(-1),'{"type":"hand_back","note":""}');
});
test('reconnect never replays takeover/input and terminal refusal clears local control',async()=>{
  const f=fixture();f.viewer.start();await f.advance(0);f.connected();f.viewer.takeControl();ack(f);f.viewer.input(click);
  f.sockets[0]?.onclose?.({code:1006});await f.advance(1000);f.connected();ack(f);f.viewer.input(click);
  assert.deepEqual(f.sockets.at(-1)?.sent,[]);
  f.viewer.takeControl();ack(f);f.sockets.at(-1)?.onmessage?.({data:'{"type":"error","error_code":12408}'});f.viewer.input(click);
  assert.match(f.states.at(-1)?.label??'',/owner is controlling/);assert.equal(f.states.at(-1)?.phase,'unavailable');
  assert.deepEqual(f.sockets.at(-1)?.sent,['{"type":"take_control"}','{"type":"hand_back","note":""}']);f.viewer.stop();
});
test('numeric authority refusals close without reconnecting and sanitized text remains the fallback',async()=>{
  for(const code of [12408,12420,12421]){
    const f=fixture();f.viewer.start();await f.advance(0);f.connected();f.viewer.takeControl();ack(f);
    f.sockets[0]?.onmessage?.({data:JSON.stringify({type:'error',error_code:code,message:'Sanitized server text'})});
    await f.advance(60_000);assert.equal(f.sockets.length,1);assert.equal(f.sockets[0]?.closed,true);
    assert.equal(f.states.at(-1)?.phase,'unavailable');assert.notEqual(f.states.at(-1)?.label,'Sanitized server text');
    f.viewer.stop();
  }
  assert.equal(refusalLabel({error_code:99999,message:'Sanitized server text'}),'Sanitized server text');
  assert.equal(refusalLabel({error_code:99999}),'Desktop permission or runtime unavailable (code 99999).');
});
test('Stop is explicit; malformed/overflow input cannot leave the client',async()=>{
  const f=fixture();f.viewer.start();await f.advance(0);f.connected();f.viewer.stopTask();assert.equal(f.sockets[0]?.sent.length,0);
  f.viewer.takeControl();ack(f);f.viewer.stopTask();assert.equal(f.sockets[0]?.sent.at(-1),'{"type":"stop"}');
  f.viewer.input({tool:'exec',arguments:{command:'anything'}});f.viewer.input({tool:'click',arguments:{...click.arguments,x:Infinity}});
  assert.equal(f.sockets[0]?.sent.length,2);
  for(let i=0;i<100;i++)f.viewer.input(click);
  assert.equal(f.sockets[0]?.sent.filter(v=>v instanceof ArrayBuffer).length,60);f.viewer.stop();
});

test('input reports acceptance so a rejected text send can preserve the draft', async () => {
  const f = fixture();
  f.viewer.start(); await f.advance(0); f.connected();
  const text = { tool: 'type_text', arguments: { text: 'Draft to keep' } };
  assert.equal(f.viewer.input(text), false, 'viewing alone cannot send text');
  f.viewer.takeControl(); ack(f);
  const socket = f.sockets[0]!;
  socket.bufferedAmount = 65537;
  assert.equal(f.viewer.input(text), false, 'backpressure must not claim text was sent');
  socket.bufferedAmount = 0;
  assert.equal(f.viewer.input({ tool: 'exec', arguments: {} }), false);
  assert.equal(f.viewer.input({ tool: 'type_text', arguments: { text: '文'.repeat(8192) } }), false);
  assert.equal(socket.closed, false, 'oversized encoded input is not a transport failure');
  assert.equal(f.viewer.input(text), true);
  for (let i = 1; i < 60; i++) assert.equal(f.viewer.input(click), true);
  assert.equal(f.viewer.input(text), false, 'rate-limited input is not accepted');
  await f.advance(1000);
  assert.equal(f.viewer.input(text), true);
  f.viewer.stop();
  assert.equal(f.viewer.input(text), false);
});

test('input transport failure reconnects without replaying text or control', async () => {
  const f = fixture();
  f.viewer.start(); await f.advance(0); f.connected();
  f.viewer.takeControl(); ack(f);
  const socket = f.sockets[0]!;
  const send = socket.send.bind(socket);
  socket.send = value => {
    if (value instanceof ArrayBuffer) throw new Error('Transport closed');
    send(value);
  };
  const text = { tool: 'type_text', arguments: { text: 'Never replay this' } };
  assert.equal(f.viewer.input(text), false);
  assert.equal(socket.closed, true);
  assert.equal(f.states.at(-1)?.phase, 'reconnecting');
  await f.advance(1000); f.connected(); ack(f);
  assert.deepEqual(f.sockets.at(-1)?.sent, []);
  assert.equal(f.viewer.input(text), false, 'a new connection requires a fresh human takeover');
  f.viewer.stop();
});
