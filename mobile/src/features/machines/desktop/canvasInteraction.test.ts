import assert from 'node:assert/strict';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';
import { canvasDocument } from './canvasDocument';

function renderer() {
  let now = 0;
  const messages: Record<string, any>[] = [];
  const handlers: Record<string, (event: any) => void> = {};
  const canvas = { width: 800, height: 600, style: {} as Record<string, string>, getContext: () => ({}),
    getBoundingClientRect: () => ({left:100,top:0,right:900,bottom:600,width:800,height:600}) };
  const window: any = { innerWidth: 1000, innerHeight: 600, addEventListener: () => {}, ReactNativeWebView: {postMessage: (s: string) => messages.push(JSON.parse(s))} };
  const html = canvasDocument();
  const opening = html.indexOf('<script>');
  const closing = html.lastIndexOf('</script>');
  assert.notEqual(opening, -1, 'generated canvas has an opening script tag');
  assert.ok(closing > opening, 'generated canvas has a closing script tag');
  const source = html.slice(opening + '<script>'.length, closing);
  runInNewContext(source, {window, document: {getElementById:()=>canvas,addEventListener:(name:string,handler:(event:any)=>void)=>{handlers[name]=handler;}}, Date: {now:()=>now}});
  const touch = (x: number, y: number) => ({clientX:x,clientY:y});
  const fire = (name: string, touches: ReturnType<typeof touch>[], changedTouches = touches) => handlers[name]!({touches,changedTouches,preventDefault(){}});
  return { window, messages, canvas, touch, fire, wait: (ms:number)=>{now+=ms;}, inputs:()=>messages.filter(v=>v.type==='input') };
}
test('canvas tap/long press/drag match web pixel inputs and letterbox rejects',()=>{
  const r=renderer();const a=r.touch(120,30),b=r.touch(160,60);
  r.fire('touchstart',[a]);r.fire('touchend',[],[a]);assert.equal(r.inputs().length,0);
  r.window.nyxDesktop.control(true);
  r.fire('touchstart',[r.touch(20,30)]);r.fire('touchend',[],[r.touch(20,30)]);assert.equal(r.inputs().length,0);
  r.fire('touchstart',[a]);r.fire('touchend',[],[a]);
  r.fire('touchstart',[a]);r.wait(600);r.fire('touchend',[],[a]);
  r.fire('touchstart',[a]);r.wait(400);r.fire('touchend',[],[b]);
  assert.deepEqual(r.inputs().map(v=>v.input),[
    {tool:'click',arguments:{x:20,y:30,button:'left',count:1}},
    {tool:'click',arguments:{x:20,y:30,button:'right',count:1}},
    {tool:'drag',arguments:{from_x:20,from_y:30,to_x:60,to_y:60,button:'left',duration_ms:400}},
  ]);
  r.fire('touchstart',[a]);r.window.nyxDesktop.control(true);r.wait(200);r.fire('touchend',[],[b]);
  assert.equal(r.inputs().at(-1)?.input.tool,'drag','heartbeat control state cannot cancel an active gesture');
  const before = r.inputs().length;
  r.fire('touchstart',[a]);r.window.nyxDesktop.control(false);r.fire('touchend',[],[b]);
  assert.equal(r.inputs().length,before,'losing control cancels the active gesture');
});
test('two-finger pan scrolls only after control; pinch changes local view only',()=>{
  const r=renderer();r.window.nyxDesktop.control(true);
  r.fire('touchstart',[r.touch(200,200),r.touch(300,200)]);r.wait(100);
  r.fire('touchmove',[r.touch(200,230),r.touch(300,230)]);r.fire('touchend',[],[r.touch(200,230)]);
  assert.deepEqual(r.inputs().map(v=>v.input),[{tool:'scroll',arguments:{x:150,y:230,direction:'up',amount:3}}]);
  r.fire('touchstart',[r.touch(200,200),r.touch(300,200)]);r.wait(100);
  r.fire('touchmove',[r.touch(150,200),r.touch(350,200)]);r.fire('touchend',[],[r.touch(150,200)]);
  assert.equal(r.canvas.style.width,'1600px');assert.equal(r.inputs().length,1,'pinch never sends computer input');
  r.window.nyxDesktop.clear(2);r.fire('touchstart',[r.touch(200,200)]);r.fire('touchend',[],[r.touch(200,200)]);
  assert.equal(r.inputs().length,1,'clear resets control gate and gesture');
});
