import assert from 'node:assert/strict';
import { test } from 'node:test';
import { base64, desktopActivity, desktopFrame, envelope, jpegSize, MAX_JPEG_BYTES } from './protocol';
import { GREEN_PATCH, RED_JPEG, SESSION, vector } from './testVectors';

const jpeg = new Uint8Array(Buffer.from(RED_JPEG, 'base64'));
const rectangle = { width: 8, height: 6, x: 2, y: 2, patchWidth: 2, patchHeight: 2, base: 1n };
test('NYXM golden header keeps kind, flag, UUID and big endian u64 independent', () => {
  const buffer = vector(jpeg, 0xfedcba9876543210n);
  const bytes = new Uint8Array(buffer);
  assert.equal(Buffer.from(bytes.subarray(0,30)).toString('hex'), '4e59584d010000112233445566778899aabbccddeefffedcba9876543210');
  assert.equal(desktopFrame(buffer, SESSION)?.sequence, 0xfedcba9876543210n);
  bytes[5] = 1; assert.equal(desktopFrame(buffer, SESSION)?.end, true);
  bytes[4] = 2; assert.equal(envelope(buffer, SESSION)?.kind, 2); assert.equal(desktopFrame(buffer, SESSION), undefined);
  bytes[5] = 2; assert.equal(envelope(buffer, SESSION), undefined);
});
test('legacy and NYXD reset/patch vectors enforce payload, UUID, dimensions and base rules', () => {
  const legacy = desktopFrame(vector(jpeg), SESSION)!;
  assert.deepEqual([legacy.width, legacy.height], [8,6]);
  const patch = desktopFrame(vector(new Uint8Array(Buffer.from(GREEN_PATCH,'base64')), 2n, rectangle), SESSION)!;
  assert.equal(patch.rectangle?.base, 1n); assert.deepEqual([patch.width,patch.height],[2,2]);
  for (const change of [{ width:1921 }, { height:1201 }, { patchWidth:0 }, { x:8 }, { y:6 }, { base:0n }])
    assert.equal(desktopFrame(vector(patch.jpeg, 2n, { ...rectangle,...change }), SESSION), undefined);
  assert.equal(desktopFrame(vector(jpeg, 3n, { ...rectangle, x:0,y:0,patchWidth:8,patchHeight:6,base:0n }), SESSION)?.rectangle?.base, 0n);
  assert.equal(desktopFrame(vector(jpeg), 'ffffffff-ffff-ffff-ffff-ffffffffffff'), undefined);
  assert.equal(desktopFrame(vector(jpeg), 'bad'), undefined);
  assert.equal(desktopFrame(vector(new Uint8Array(MAX_JPEG_BYTES+1)), SESSION), undefined);
  assert.equal(desktopFrame(vector(jpeg).slice(0,29), SESSION), undefined);
  for (const [offset,value] of [[0,0],[4,12],[5,9],[6,1]] as const) {
    const buffer = vector(jpeg); new Uint8Array(buffer)[offset] = value;
    assert.equal(desktopFrame(buffer, SESSION), undefined);
  }
  assert.equal(desktopFrame(vector(jpeg).slice(0,40), SESSION), undefined);
  assert.equal(desktopFrame(vector(jpeg, 0n), SESSION), undefined);
  assert.equal(desktopFrame(vector(jpeg, 2n, rectangle), SESSION), undefined, 'JPEG/header decoded-size mismatch');
  const huge = jpeg.slice();
  const sof = huge.findIndex((value,index) => value === 255 && [192,194].includes(huge[index+1]!));
  huge[sof+7]=255; huge[sof+8]=255;
  assert.equal(jpegSize(huge), undefined, 'pre-decode bounds apply to legacy full frames');
});
test('activity is bounded metadata and base64 roundtrips without Blob/Buffer in production', () => {
  const bytes = new Uint8Array(vector(new TextEncoder().encode('{"tool":"click","cursor":{"x":0.5,"y":0}}')));
  bytes[4]=11; assert.deepEqual(desktopActivity(bytes.buffer,SESSION),{tool:'click'});
  const bad = new Uint8Array(vector(new TextEncoder().encode('{"tool":"click","cursor":{"x":2,"y":0}}')));
  bad[4]=11; assert.equal(desktopActivity(bad.buffer,SESSION),undefined);
  for (const length of [0,1,2,3,12287,12288,12289]) {
    const bytes = Uint8Array.from({length},(_,i)=>i%256);
    assert.equal(base64(bytes),Buffer.from(bytes).toString('base64'));
  }
});

test('native input matches web UTF-8/UUID/u64 wire contract and strips renderer authority fields', async () => {
  const { desktopInput, desktopInputValue } = await import('./protocol');
  const input = desktopInputValue({ tool:'type_text',arguments:{text:'Hi 世界 😀',revision:999,viewer_id:'forged'},revision:999 });
  assert.deepEqual(input,{tool:'type_text',arguments:{text:'Hi 世界 😀'}});
  const frame = desktopInput(SESSION, 0x0102030405060708n, input!);
  const bytes = new Uint8Array(frame);
  assert.deepEqual([...bytes.slice(0,6)],[78,89,88,77,2,0]);
  assert.equal(Buffer.from(bytes.slice(6,22)).toString('hex'),SESSION.replaceAll('-',''));
  assert.equal(new DataView(frame).getBigUint64(22),0x0102030405060708n);
  assert.deepEqual(JSON.parse(new TextDecoder().decode(bytes.slice(30))),input);
  assert.equal(desktopInputValue({tool:'take_control',arguments:{}}),undefined);
  assert.equal(desktopInputValue({tool:'press_key',arguments:{key:'EXEC'}}),undefined);
  assert.equal(desktopInputValue({tool:'click',arguments:{x:1,y:1201,button:'left',count:1}}),undefined);
  const forged = { ...input!, viewer_id: 'forged', revision: 999 };
  const safeBytes = new Uint8Array(desktopInput(SESSION, 1n, forged));
  assert.deepEqual(JSON.parse(new TextDecoder().decode(safeBytes.slice(30))),input);
  assert.throws(()=>desktopInput(SESSION,0n,input!));
  assert.throws(()=>desktopInput(SESSION,1n,{tool:'type_text',arguments:{text:'界'.repeat(8192)}}));
});
