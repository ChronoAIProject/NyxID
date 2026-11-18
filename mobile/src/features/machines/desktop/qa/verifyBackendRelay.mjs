// Real local-backend/source smoke check; no simulator, Metro or frame files.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { desktopFrame, desktopInput } from '../protocol.ts';
import { relayFixture } from './relayFixture.mjs';

const require = createRequire(new URL('../../../../../package.json', import.meta.url));
const WebSocket = require('ws');
const fixture = await relayFixture();
let socket;
try {
  let session;
  let states = 0;
  let controls = false;
  const frames = [];
  let failure;
  socket = new WebSocket(
    `ws://127.0.0.1:3011/api/v1/assistant/nyxagent/machines/${fixture.params.nodeId}/desktop`,
    { headers: { Authorization: `Bearer ${fixture.session.accessToken}` } },
  );
  socket.on('error', error => { failure = error; });
  socket.on('message', (bytes, binary) => {
    try {
      if (!binary) {
        const value = JSON.parse(bytes.toString());
        if (value.type === 'connected') session = value.session_id;
        if (value.type === 'state') {states++;controls=value.controls===true;}
        assert.notEqual(value.type, 'error', 'Backend refused the viewer');
        return;
      }
      const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
      const frame = desktopFrame(buffer, session);
      assert.ok(frame, 'Production relay forwards a valid desktop frame');
      frames.push(frame);
    } catch (error) { failure = error; }
  });
  const wait = async predicate => {
    const deadline = Date.now() + 20000;
    while (!predicate()) {
      if (failure) throw failure;
      if (Date.now() >= deadline) throw new Error('Production backend relay deadline');
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    if (failure) throw failure;
  };
  await wait(() => frames.length >= 3);
  assert.deepEqual(frames.slice(0, 3).map(frame => frame.sequence), [1n, 2n, 3n]);
  assert.equal(frames[0].rectangle, undefined);
  assert.equal(frames[1].rectangle.base, 1n);
  assert.equal(frames[2].rectangle, undefined);
  await wait(() => states >= 2); // Immediate state plus the next ten-second heartbeat.
  assert.equal(frames.length, 3, 'Idle desktop stays connected without new frames');
  socket.send('{"type":"refresh_frame"}');
  await wait(() => frames.length >= 6);
  assert.deepEqual(frames.slice(3, 6).map(frame => frame.sequence), [4n, 5n, 6n]);
  assert.equal(fixture.metrics.inputs,0,'viewing sends no input');
  socket.send('{"type":"take_control"}');
  await wait(()=>controls);
  assert.equal(fixture.metrics.controlChanges,1);
  for(const [index,input] of [
    {tool:'click',arguments:{x:1,y:1,button:'left',count:1}},
    {tool:'drag',arguments:{from_x:1,from_y:1,to_x:3,to_y:3,button:'left',duration_ms:300}},
    {tool:'scroll',arguments:{x:1,y:1,direction:'down',amount:3}},
    {tool:'type_text',arguments:{text:'synthetic'}},
    {tool:'press_key',arguments:{key:'ENTER'}},
  ].entries()) socket.send(desktopInput(session,BigInt(index+1),input));
  await wait(()=>fixture.metrics.inputs===5);
  socket.send('{"type":"hand_back","note":""}');await wait(()=>!controls&&fixture.metrics.handBacks===1);
  socket.close();
  await new Promise(resolve => setTimeout(resolve, 200));
  assert.equal(fixture.metrics.stops,0,'hand-back/close never stop task');
  assert.deepEqual(fixture.metrics.forbiddenOperations, []);
  console.log(JSON.stringify({ frames: frames.length, stateHeartbeats: states, source: fixture.metrics }));
} finally {
  socket?.close();
  fixture.close();
}
