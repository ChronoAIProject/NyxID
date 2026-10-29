import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createCompositor, type CanvasFrame } from './compositor';
import { desktopFrame } from './protocol';
import { GREEN_PATCH, RED_JPEG, SESSION, vector } from './testVectors';

const full = (sequence: bigint) => desktopFrame(vector(new Uint8Array(Buffer.from(RED_JPEG,'base64')),sequence), SESSION)!;
const patch = (sequence: bigint,base: bigint) => desktopFrame(vector(new Uint8Array(Buffer.from(GREEN_PATCH,'base64')),sequence,
  {width:8,height:6,x:2,y:2,patchWidth:2,patchHeight:2,base}), SESSION)!;
test('one decode at a time, total queue <=4, replay/base validation and full reset', () => {
  const sent: CanvasFrame[]=[]; let refresh=0;
  const c=createCompositor(f=>sent.push(f),()=>refresh++);
  c.enqueue(full(1n)); const generation=sent[0]!.generation; c.enqueue(patch(2n,1n)); c.enqueue(patch(3n,2n)); c.enqueue(patch(4n,3n)); c.enqueue(patch(5n,4n));
  assert.equal(c.depth(),4); assert.equal(sent.length,1); assert.equal(refresh,1);
  c.acknowledge({generation,sequence:'1',ok:true}); assert.equal(sent.length,2);
  c.acknowledge({generation,sequence:'2',ok:true}); c.acknowledge({generation,sequence:'3',ok:true}); c.acknowledge({generation,sequence:'4',ok:true});
  c.enqueue(patch(6n,5n)); assert.equal(c.depth(),0); assert.equal(refresh,2);
  c.enqueue(full(7n)); c.acknowledge({generation,sequence:'7',ok:true});
  c.enqueue(full(6n)); c.enqueue(patch(7n,7n)); assert.equal(c.depth(),0);
  c.enqueue(full(8n)); c.enqueue(patch(9n,8n)); c.enqueue(full(10n));
  assert.equal(c.depth(),2); c.acknowledge({generation,sequence:'8',ok:true}); assert.equal(sent.at(-1)?.sequence,'10');
});
test('disposal invalidates decode callbacks and corruption requires a full frame', () => {
  const sent: CanvasFrame[]=[]; let refresh=0;
  const c=createCompositor(f=>sent.push(f),()=>refresh++);
  c.enqueue(full(1n)); const generation=sent[0]!.generation; c.enqueue(patch(2n,1n));
  c.acknowledge({generation,sequence:'1',ok:false}); assert.equal(c.depth(),0); assert.equal(refresh,1);
  const nextGeneration=c.reset(); c.enqueue(full(1n));
  assert.equal(c.acknowledge({generation,sequence:'1',ok:true}),false); assert.equal(c.depth(),1);
  assert.equal(c.acknowledge({generation:nextGeneration,sequence:'1',ok:true}),true); assert.equal(c.depth(),0);
  c.reset(); assert.equal(c.depth(),0);
});
